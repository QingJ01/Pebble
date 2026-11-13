use super::*;

impl GmailProvider {
    /// Gmail labels are independent resources, including labels containing '/'.
    /// Only leaf-label rename is offered; there is no atomic subtree rename API.
    pub async fn create_folder_label(&self, name: &str) -> Result<Folder> {
        self.write_folder_label(None, name).await
    }

    pub async fn rename_folder_label(&self, id: &str, name: &str) -> Result<Folder> {
        self.write_folder_label(Some(id), name).await
    }

    async fn write_folder_label(&self, id: Option<&str>, name: &str) -> Result<Folder> {
        let mut url = reqwest::Url::parse(&format!("{GMAIL_API_BASE}/labels")).unwrap();
        if let Some(id) = id {
            url.path_segments_mut().unwrap().push(id);
        }
        #[cfg(test)]
        let url = reqwest::Url::parse(&self.test_request_url(url.as_str())).unwrap();
        let response = self
            .client
            .request(
                if id.is_some() {
                    reqwest::Method::PATCH
                } else {
                    reqwest::Method::POST
                },
                url,
            )
            .bearer_auth(self.token())
            .json(&serde_json::json!({"name": name}))
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Gmail label request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(PebbleError::Network(format!(
                "Gmail label request failed (status {})",
                response.status()
            )));
        }
        let label: GmailLabel = response
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Invalid Gmail label response: {e}")))?;
        if label.id.is_empty() || label.name != name || id.is_some_and(|id| id != label.id) {
            return Err(PebbleError::Network(
                "Gmail returned an unexpected label".into(),
            ));
        }
        Ok(gmail_label_to_folder(&label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(
        responses: Vec<(u16, &'static str)>,
    ) -> (
        String,
        std::thread::JoinHandle<Vec<(String, serde_json::Value)>>,
    ) {
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
                requests.push((first, serde_json::from_slice(&bytes).unwrap()));
                write!(stream,"HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (endpoint, handle)
    }
    #[tokio::test]
    async fn label_create_and_rename_use_server_ids_and_exact_methods() {
        let (endpoint, server) = server(vec![
            (200, r#"{"id":"Label_1","name":"Parent/子","type":"user"}"#),
            (200, r#"{"id":"Label_1","name":"Parent/New","type":"user"}"#),
        ]);
        let provider = GmailProvider::new("fixture".into());
        provider.set_test_api_endpoint(endpoint);
        assert_eq!(
            provider
                .create_folder_label("Parent/子")
                .await
                .unwrap()
                .remote_id,
            "Label_1"
        );
        assert_eq!(
            provider
                .rename_folder_label("Label_1", "Parent/New")
                .await
                .unwrap()
                .remote_id,
            "Label_1"
        );
        let requests = server.join().unwrap();
        assert!(requests[0].0.starts_with("POST /gmail/v1/users/me/labels "));
        assert!(requests[1]
            .0
            .starts_with("PATCH /gmail/v1/users/me/labels/Label_1 "));
        assert_eq!(requests[0].1["name"], "Parent/子");
    }
    #[tokio::test]
    async fn label_failures_and_unexpected_server_identity_are_rejected() {
        let (endpoint, server) = server(vec![
            (403, r#"{"error":"forbidden"}"#),
            (200, r#"{"id":"different","name":"New","type":"user"}"#),
        ]);
        let provider = GmailProvider::new("fixture".into());
        provider.set_test_api_endpoint(endpoint);
        assert!(provider.create_folder_label("New").await.is_err());
        assert!(provider
            .rename_folder_label("Label_1", "New")
            .await
            .is_err());
        server.join().unwrap();
    }
    #[tokio::test]
    async fn move_modification_removes_only_the_explicit_source_label() {
        let (endpoint, server) = server(vec![(200, "{}")]);
        let provider = GmailProvider::new("fixture".into());
        provider.set_test_api_endpoint(endpoint);
        provider
            .modify_labels("message", &["Label_Target".into()], &["INBOX".into()])
            .await
            .unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0]
            .0
            .starts_with("POST /gmail/v1/users/me/messages/message/modify "));
        assert_eq!(
            requests[0].1,
            serde_json::json!({"addLabelIds":["Label_Target"],"removeLabelIds":["INBOX"]})
        );
    }
}
