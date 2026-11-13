use super::*;

fn folder_url(id: Option<&str>, create: bool) -> reqwest::Url {
    let mut url = reqwest::Url::parse(&format!("{GRAPH_API_BASE}/mailFolders")).unwrap();
    if let Some(id) = id {
        url.path_segments_mut().unwrap().push(id);
        if create {
            url.path_segments_mut().unwrap().push("childFolders");
        }
    }
    url
}

impl OutlookProvider {
    /// A destination Message-ID match is recovery evidence only when the
    /// original resource is gone, not when the destination holds another copy.
    pub async fn recover_managed_move(
        &self,
        source_id: &str,
        target: &str,
        message_id: &str,
    ) -> Result<Option<String>> {
        let mut url = reqwest::Url::parse(&format!("{GRAPH_API_BASE}/messages")).unwrap();
        url.path_segments_mut().unwrap().push(source_id);
        url.query_pairs_mut().append_pair("$select", "id");
        let response = self.get(url.as_str()).await?;
        if response.status().is_success() {
            return Ok(None);
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return self.find_managed_message(target, message_id).await;
        }
        Err(PebbleError::Network(format!(
            "Graph move source check failed (status {})",
            response.status()
        )))
    }

    #[cfg(test)]
    pub(super) fn test_folder_url(&self, url: &str) -> String {
        let endpoint = self.test_folder_endpoint.read().unwrap().clone();
        let Some(endpoint) = endpoint else {
            return url.to_owned();
        };
        let mut url = reqwest::Url::parse(url).unwrap();
        let endpoint = reqwest::Url::parse(&endpoint).unwrap();
        url.set_scheme(endpoint.scheme()).unwrap();
        url.set_host(endpoint.host_str()).unwrap();
        url.set_port(endpoint.port()).unwrap();
        url.to_string()
    }

    /// Internet Message-ID remains stable when Graph's move changes the resource ID.
    /// Refuse ambiguous matches rather than changing the wrong cached message.
    pub async fn find_managed_message(
        &self,
        target: &str,
        message_id: &str,
    ) -> Result<Option<String>> {
        if message_id.contains(['\r', '\n']) {
            return Err(PebbleError::Validation("Invalid Message-ID".into()));
        }
        let mut url = folder_url(Some(target), false);
        url.path_segments_mut().unwrap().push("messages");
        url.query_pairs_mut()
            .append_pair(
                "$filter",
                &format!("internetMessageId eq '{}'", message_id.replace('\'', "''")),
            )
            .append_pair("$select", "id,internetMessageId")
            .append_pair("$top", "2");
        let response = self.get(url.as_str()).await?;
        if !response.status().is_success() {
            return Err(PebbleError::Network(format!(
                "Graph move reconciliation failed (status {})",
                response.status()
            )));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Invalid Graph message response: {e}")))?;
        let rows = value["value"]
            .as_array()
            .ok_or_else(|| PebbleError::Network("Graph returned no message list".into()))?;
        if rows.len() > 1 || value.get("@odata.nextLink").is_some() {
            return Err(PebbleError::Sync(
                "Multiple destination messages match this Message-ID".into(),
            ));
        }
        match rows.first() {
            None => Ok(None),
            Some(row) if row["internetMessageId"].as_str() == Some(message_id) => row["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(|id| Some(id.to_owned()))
                .ok_or_else(|| PebbleError::Network("Graph returned an empty message ID".into())),
            _ => Err(PebbleError::Network(
                "Graph returned an unexpected message".into(),
            )),
        }
    }
    /// Graph's root endpoint is not recursive. Follow children and pagination;
    /// only trusted Graph continuation URLs may receive the bearer token.
    pub async fn list_managed_folders(&self) -> Result<Vec<Folder>> {
        #[derive(Deserialize)]
        struct Page {
            value: Vec<GraphFolder>,
            #[serde(rename = "@odata.nextLink")]
            next: Option<String>,
        }
        let mut pending = std::collections::VecDeque::from([(
            format!("{GRAPH_API_BASE}/mailFolders?$top=100"),
            None,
        )]);
        let mut visited = std::collections::HashSet::new();
        let mut folders = Vec::new();
        while let Some((url, parent)) = pending.pop_front() {
            if !visited.insert(url.clone()) || visited.len() > 10_000 {
                return Err(PebbleError::Network(
                    "Invalid Graph folder pagination".into(),
                ));
            }
            validate_graph_continuation_url(&url)?;
            let response = self.get(&url).await?;
            if !response.status().is_success() {
                return Err(PebbleError::Network(format!(
                    "Graph folder listing failed (status {})",
                    response.status()
                )));
            }
            let page: Page = response
                .json()
                .await
                .map_err(|e| PebbleError::Network(format!("Invalid Graph folder page: {e}")))?;
            if let Some(next) = page.next {
                validate_graph_continuation_url(&next)?;
                pending.push_back((next, parent.clone()));
            }
            for entry in page.value {
                if should_hide_outlook_folder(
                    entry.display_name.as_deref(),
                    entry.well_known_name.as_deref(),
                ) {
                    continue;
                }
                if entry.child_folder_count.unwrap_or(0) > 0 {
                    let mut url = folder_url(Some(&entry.id), true);
                    url.set_query(Some("$top=100"));
                    pending.push_back((url.to_string(), Some(entry.id.clone())));
                }
                let mut folder = graph_folder_to_folder(&entry, &self.account_id);
                // Resolve system roles by Graph identity below, never by a
                // user-controlled display name such as "Sent projects".
                folder.role = None;
                folder.is_system = false;
                folder.sort_order = crate::imap::folder_sort_order(&None);
                folder.parent_id = parent.clone();
                folders.push(folder);
            }
        }
        // Opaque IDs and localized display names cannot identify system folders.
        let mut hidden = Vec::new();
        for (alias, role) in [
            ("inbox", Some(pebble_core::FolderRole::Inbox)),
            ("drafts", Some(pebble_core::FolderRole::Drafts)),
            ("sentitems", Some(pebble_core::FolderRole::Sent)),
            ("deleteditems", Some(pebble_core::FolderRole::Trash)),
            ("junkemail", Some(pebble_core::FolderRole::Spam)),
            ("archive", Some(pebble_core::FolderRole::Archive)),
            ("outbox", None),
            ("conversationhistory", None),
        ] {
            let response = self
                .get(&format!("{GRAPH_API_BASE}/mailFolders/{alias}?$select=id"))
                .await?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                continue;
            }
            if !response.status().is_success() {
                return Err(PebbleError::Network(format!(
                    "Graph system folder lookup failed (status {})",
                    response.status()
                )));
            }
            let value: serde_json::Value = response
                .json()
                .await
                .map_err(|e| PebbleError::Network(format!("Invalid Graph folder response: {e}")))?;
            let id = value["id"]
                .as_str()
                .ok_or_else(|| PebbleError::Network("Graph system folder has no ID".into()))?;
            if let Some(folder) = folders.iter_mut().find(|f| f.remote_id == id) {
                folder.is_system = true;
                folder.role = role;
                folder.sort_order = crate::imap::folder_sort_order(&folder.role);
                if folder.role.is_none() {
                    hidden.push(folder.remote_id.clone());
                }
            }
        }
        while let Some(parent) = hidden.pop() {
            folders.retain(|folder| {
                if folder.parent_id.as_deref() == Some(&parent) {
                    hidden.push(folder.remote_id.clone());
                    false
                } else {
                    folder.remote_id != parent
                }
            });
        }
        Ok(folders)
    }

    pub async fn create_managed_folder(
        &self,
        parent_remote_id: Option<&str>,
        name: &str,
    ) -> Result<Folder> {
        self.write_managed_folder(parent_remote_id, name, true)
            .await
    }

    pub async fn rename_managed_folder(&self, remote_id: &str, name: &str) -> Result<Folder> {
        self.write_managed_folder(Some(remote_id), name, false)
            .await
    }

    async fn write_managed_folder(
        &self,
        id: Option<&str>,
        name: &str,
        create: bool,
    ) -> Result<Folder> {
        let url = folder_url(id, create);
        #[cfg(test)]
        let url = self.test_folder_url(url.as_str());
        let response = self
            .client
            .request(
                if create {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::PATCH
                },
                url,
            )
            .bearer_auth(self.token())
            .json(&serde_json::json!({"displayName":name}))
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Graph folder request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(PebbleError::Network(format!(
                "Graph folder request failed (status {})",
                response.status()
            )));
        }
        let folder: GraphFolder = response
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Invalid Graph folder response: {e}")))?;
        if folder.id.is_empty()
            || folder.display_name.as_deref() != Some(name)
            || (!create && id != Some(folder.id.as_str()))
        {
            return Err(PebbleError::Network(
                "Graph returned an unexpected folder".into(),
            ));
        }
        let mut folder = graph_folder_to_folder(&folder, &self.account_id);
        folder.role = None;
        folder.is_system = false;
        folder.sort_order = crate::imap::folder_sort_order(&None);
        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_ids_are_one_url_segment() {
        let url = folder_url(Some("A/B?#"), true);
        assert!(url.as_str().ends_with("/A%2FB%3F%23/childFolders"));
        assert!(folder_url(Some("opaque"), false)
            .as_str()
            .ends_with("/mailFolders/opaque"));
        assert!(folder_url(None, true).as_str().ends_with("/mailFolders"));
    }

    fn server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            len = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut bytes = vec![0; len];
                reader.read_exact(&mut bytes).unwrap();
                requests.push(format!("{}{}", first, String::from_utf8(bytes).unwrap()));
                write!(stream,"HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (endpoint, handle)
    }
    fn provider(endpoint: String) -> OutlookProvider {
        let provider = OutlookProvider::new("fixture".into(), "a".into());
        *provider.test_folder_endpoint.write().unwrap() = Some(endpoint);
        provider
    }
    #[tokio::test]
    async fn graph_create_child_and_rename_preserve_opaque_ids() {
        let (endpoint, server) = server(vec![
            (201, r#"{"id":"A/B?#","displayName":"New"}"#),
            (200, r#"{"id":"A/B?#","displayName":"Renamed"}"#),
            (403, "{}"),
        ]);
        let provider = provider(endpoint);
        assert_eq!(
            provider
                .create_managed_folder(Some("Parent/Id"), "New")
                .await
                .unwrap()
                .remote_id,
            "A/B?#"
        );
        assert_eq!(
            provider
                .rename_managed_folder("A/B?#", "Renamed")
                .await
                .unwrap()
                .remote_id,
            "A/B?#"
        );
        assert!(provider
            .create_managed_folder(None, "Denied")
            .await
            .is_err());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /v1.0/me/mailFolders/Parent%2FId/childFolders "));
        assert!(requests[1].starts_with("PATCH /v1.0/me/mailFolders/A%2FB%3F%23 "));
        assert!(requests[1].contains(r#""displayName":"Renamed""#));
    }
    #[tokio::test]
    async fn graph_discovers_nested_paged_folders_and_localized_system_ids() {
        let (endpoint, server) = server(vec![
            (
                200,
                r#"{"value":[{"id":"parent","displayName":"Sent projects","childFolderCount":1},{"id":"opaque-inbox","displayName":"Boîte de réception"},{"id":"opaque-outbox","displayName":"Boîte d’envoi"}],"@odata.nextLink":"https://graph.microsoft.com/v1.0/me/mailFolders?$skiptoken=next"}"#,
            ),
            (200, r#"{"value":[]}"#),
            (200, r#"{"value":[{"id":"child","displayName":"Nested"}]}"#),
            (200, r#"{"id":"opaque-inbox"}"#),
            (404, "{}"),
            (404, "{}"),
            (404, "{}"),
            (404, "{}"),
            (404, "{}"),
            (200, r#"{"id":"opaque-outbox"}"#),
            (404, "{}"),
        ]);
        let provider = provider(endpoint);
        let folders = provider.list_managed_folders().await.unwrap();
        assert!(folders
            .iter()
            .any(|f| f.remote_id == "child" && f.parent_id.as_deref() == Some("parent")));
        assert!(folders.iter().any(|f| f.remote_id == "opaque-inbox"
            && f.is_system
            && f.role == Some(FolderRole::Inbox)));
        assert!(folders
            .iter()
            .any(|f| f.remote_id == "parent" && !f.is_system && f.role.is_none()));
        assert!(folders.iter().all(|f| f.remote_id != "opaque-outbox"));
        assert_eq!(server.join().unwrap().len(), 11);
    }
    #[tokio::test]
    async fn graph_rejects_untrusted_pagination_before_following_it() {
        let (endpoint, server) = server(vec![(
            200,
            r#"{"value":[],"@odata.nextLink":"https://evil.invalid/mailFolders"}"#,
        )]);
        let provider = provider(endpoint);
        assert!(provider.list_managed_folders().await.is_err());
        assert_eq!(server.join().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn graph_only_adopts_destination_copy_when_original_resource_is_gone() {
        let (endpoint, server) = server(vec![
            (200, r#"{"id":"source/id"}"#),
            (403, "{}"),
            (404, "{}"),
            (
                200,
                r#"{"value":[{"id":"moved-id","internetMessageId":"<m@example.invalid>"}]}"#,
            ),
        ]);
        let provider = provider(endpoint);
        assert!(provider
            .recover_managed_move("source/id", "target", "<m@example.invalid>")
            .await
            .unwrap()
            .is_none());
        assert!(provider
            .recover_managed_move("source/id", "target", "<m@example.invalid>")
            .await
            .is_err());
        assert_eq!(
            provider
                .recover_managed_move("source/id", "target", "<m@example.invalid>")
                .await
                .unwrap()
                .as_deref(),
            Some("moved-id")
        );
        let requests = server.join().unwrap();
        assert!(requests[..3]
            .iter()
            .all(|request| request.starts_with("GET /v1.0/me/messages/source%2Fid?")));
        assert!(requests[3].starts_with("GET /v1.0/me/mailFolders/target/messages?"));
    }

    #[tokio::test]
    async fn graph_recovers_moved_identity_but_rejects_ambiguous_matches() {
        let (endpoint, server) = server(vec![
            (
                200,
                r#"{"value":[{"id":"new-opaque-id","internetMessageId":"<m'@example.invalid>"}]}"#,
            ),
            (200, r#"{"value":[{},{}]}"#),
        ]);
        let provider = provider(endpoint);
        assert_eq!(
            provider
                .find_managed_message("A/B?#", "<m'@example.invalid>")
                .await
                .unwrap()
                .as_deref(),
            Some("new-opaque-id")
        );
        assert!(provider
            .find_managed_message("A/B?#", "<m'@example.invalid>")
            .await
            .is_err());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /v1.0/me/mailFolders/A%2FB%3F%23/messages?"));
        let first = requests[0]
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = reqwest::Url::parse(&format!("http://localhost{first}")).unwrap();
        assert!(url.query_pairs().any(|(key, value)| key == "$filter"
            && value == "internetMessageId eq '<m''@example.invalid>'"));
    }
}
