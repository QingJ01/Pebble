pub mod schema;

use std::path::Path;
use std::sync::Mutex;

use pebble_core::{Message, PebbleError, Result};
use pebble_core::traits::SearchHit;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::Value;
use tantivy::schema::Schema;
use tantivy::{DateTime, Index, IndexWriter, ReloadPolicy, Term, TantivyDocument};

use schema::{build_schema, SearchSchema};

const SNIPPET_MAX_LEN: usize = 150;

fn make_snippet(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    let body = doc
        .get_first(field)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if body.len() > SNIPPET_MAX_LEN {
        format!("{}…", &body[..body.floor_char_boundary(SNIPPET_MAX_LEN)])
    } else {
        body.to_string()
    }
}

pub struct AdvancedSearchParams<'a> {
    pub text: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub has_attachment: Option<bool>,
    pub folder_id: Option<&'a str>,
    pub limit: usize,
}

pub struct TantivySearch {
    index: Index,
    writer: Mutex<IndexWriter>,
    schema: SearchSchema,
    reader: tantivy::IndexReader,
    needs_reindex: bool,
}

impl TantivySearch {
    pub fn open(index_path: &Path) -> Result<Self> {
        let ss = build_schema();

        let create_fresh = |path: &Path, schema: &Schema| -> Result<Index> {
            let _ = std::fs::remove_dir_all(path);
            std::fs::create_dir_all(path)
                .map_err(|e| PebbleError::Storage(format!("Failed to create index dir: {e}")))?;
            let idx = Index::create_in_dir(path, schema.clone())
                .map_err(|e| PebbleError::Storage(format!("Failed to create index: {e}")))?;
            schema::register_tokenizers(&idx);
            Ok(idx)
        };

        let mut needs_reindex = false;
        let index = if index_path.exists() {
            match Index::open_in_dir(index_path) {
                Ok(idx) => {
                    schema::register_tokenizers(&idx);

                    // Check if schema needs rebuild: body_text must be stored and
                    // use the default tokenizer (previously used n-gram which causes bloat).
                    let existing_schema = idx.schema();
                    let needs_rebuild = match existing_schema.get_field("body_text") {
                        Ok(f) => {
                            let entry = existing_schema.get_field_entry(f);
                            if !entry.is_stored() {
                                true
                            } else {
                                match entry.field_type() {
                                    tantivy::schema::FieldType::Str(text_opts) => {
                                        match text_opts.get_indexing_options() {
                                            Some(idx_opts) => idx_opts.tokenizer() != schema::BODY_TOKENIZER,
                                            None => true,
                                        }
                                    }
                                    _ => true,
                                }
                            }
                        }
                        Err(_) => true,
                    };

                    if needs_rebuild {
                        tracing::info!("Search index schema outdated, rebuilding...");
                        needs_reindex = true;
                        create_fresh(index_path, &ss.schema)?
                    } else {
                        idx
                    }
                }
                Err(_) => create_fresh(index_path, &ss.schema)?,
            }
        } else {
            create_fresh(index_path, &ss.schema)?
        };

        let writer = index
            .writer(50_000_000)
            .map_err(|e| PebbleError::Internal(format!("Failed to create writer: {e}")))?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| PebbleError::Internal(format!("Failed to create reader: {e}")))?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            schema: ss,
            reader,
            needs_reindex,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let ss = build_schema();
        let index = Index::create_in_ram(ss.schema.clone());
        schema::register_tokenizers(&index);

        let writer = index
            .writer(15_000_000)
            .map_err(|e| PebbleError::Internal(format!("Failed to create writer: {e}")))?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| PebbleError::Internal(format!("Failed to create reader: {e}")))?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            schema: ss,
            reader,
            needs_reindex: false,
        })
    }

    /// Returns true if the index was rebuilt due to schema changes and needs re-population.
    pub fn needs_reindex(&self) -> bool {
        self.needs_reindex
    }

    /// Returns the number of documents in the index.
    pub fn doc_count(&self) -> u64 {
        let searcher = self.reader.searcher();
        searcher.num_docs()
    }

    pub fn index_message(&self, msg: &Message, folder_ids: &[String]) -> Result<()> {
        let ss = &self.schema;
        let mut doc = TantivyDocument::default();

        doc.add_text(ss.message_id, &msg.id);
        doc.add_text(ss.subject, &msg.subject);
        doc.add_text(ss.body_text, &msg.body_text);
        doc.add_text(ss.from_address, &msg.from_address);
        doc.add_text(ss.from_name, &msg.from_name);

        let to_text: Vec<String> = msg
            .to_list
            .iter()
            .chain(msg.cc_list.iter())
            .chain(msg.bcc_list.iter())
            .map(|ea| {
                if let Some(name) = &ea.name {
                    format!("{} {}", name, ea.address)
                } else {
                    ea.address.clone()
                }
            })
            .collect();
        doc.add_text(ss.to_addresses, to_text.join(" "));

        doc.add_date(ss.date, DateTime::from_timestamp_secs(msg.date));

        for fid in folder_ids {
            doc.add_text(ss.folder_id, fid);
        }
        doc.add_text(ss.account_id, &msg.account_id);
        doc.add_text(
            ss.has_attachment,
            if msg.has_attachments { "true" } else { "false" },
        );

        let writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;

        writer.delete_term(Term::from_field_text(ss.message_id, &msg.id));
        writer
            .add_document(doc)
            .map_err(|e| PebbleError::Internal(format!("Failed to add document: {e}")))?;

        Ok(())
    }

    pub fn index_messages_batch(&self, messages: &[(Message, Vec<String>)]) -> Result<()> {
        let ss = &self.schema;
        let writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;

        for (msg, folder_ids) in messages {
            let mut doc = TantivyDocument::default();
            doc.add_text(ss.message_id, &msg.id);
            doc.add_text(ss.subject, &msg.subject);
            doc.add_text(ss.body_text, &msg.body_text);
            doc.add_text(ss.from_address, &msg.from_address);
            doc.add_text(ss.from_name, &msg.from_name);

            let to_text: Vec<String> = msg.to_list.iter()
                .chain(msg.cc_list.iter())
                .chain(msg.bcc_list.iter())
                .map(|ea| match &ea.name {
                    Some(name) => format!("{} {}", name, ea.address),
                    None => ea.address.clone(),
                })
                .collect();
            doc.add_text(ss.to_addresses, to_text.join(" "));
            doc.add_date(ss.date, DateTime::from_timestamp_secs(msg.date));
            for fid in folder_ids {
                doc.add_text(ss.folder_id, fid);
            }
            doc.add_text(ss.account_id, &msg.account_id);
            doc.add_text(ss.has_attachment, if msg.has_attachments { "true" } else { "false" });

            writer.delete_term(Term::from_field_text(ss.message_id, &msg.id));
            writer.add_document(doc)
                .map_err(|e| PebbleError::Internal(format!("Failed to add document: {e}")))?;
        }
        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;

        writer
            .commit()
            .map_err(|e| PebbleError::Internal(format!("Failed to commit: {e}")))?;

        // Force the cached reader to pick up the newly committed segments immediately
        self.reader
            .reload()
            .map_err(|e| PebbleError::Internal(format!("Failed to reload reader: {e}")))?;

        Ok(())
    }

    pub fn remove_message(&self, message_id: &str) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;
        writer.delete_term(Term::from_field_text(self.schema.message_id, message_id));
        Ok(())
    }

    /// Remove all documents for an account from the search index.
    pub fn delete_by_account(&self, account_id: &str) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;
        writer.delete_term(Term::from_field_text(self.schema.account_id, account_id));
        writer
            .commit()
            .map_err(|e| PebbleError::Internal(format!("Failed to commit: {e}")))?;
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| PebbleError::Internal(format!("Failed to reload reader: {e}")))?;
        Ok(())
    }

    pub fn search(&self, query_text: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let ss = &self.schema;

        let searcher = self.reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![ss.subject, ss.body_text, ss.from_address, ss.from_name],
        );

        let query = query_parser
            .parse_query(query_text)
            .map_err(|e| PebbleError::Internal(format!("Failed to parse query: {e}")))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| PebbleError::Internal(format!("Search failed: {e}")))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_addr)
                .map_err(|e| PebbleError::Internal(format!("Failed to retrieve doc: {e}")))?;

            let message_id = doc
                .get_first(ss.message_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let snippet = make_snippet(&doc, ss.body_text);

            let subject = doc.get_first(ss.subject).and_then(|v| v.as_str()).map(|s| s.to_string());
            let from_address = doc.get_first(ss.from_address).and_then(|v| v.as_str()).map(|s| s.to_string());
            let date = doc.get_first(ss.date).and_then(|v| v.as_datetime()).map(|dt| dt.into_timestamp_secs());

            hits.push(SearchHit {
                message_id,
                score,
                snippet,
                subject,
                from_address,
                date,
            });
        }

        Ok(hits)
    }

    pub fn advanced_search(
        &self,
        params: AdvancedSearchParams<'_>,
    ) -> Result<Vec<SearchHit>> {
        let AdvancedSearchParams { text, from, to, subject, date_from, date_to, has_attachment, folder_id, limit } = params;
        let ss = &self.schema;

        let searcher = self.reader.searcher();

        let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Helper: parse a text query against specific fields
        let parse_text_query = |fields: Vec<tantivy::schema::Field>, q: &str| -> Result<Box<dyn Query>> {
            let parser = QueryParser::for_index(&self.index, fields);
            parser
                .parse_query(q)
                .map_err(|e| PebbleError::Internal(format!("Failed to parse query: {e}")))
        };

        if let Some(q) = text {
            if !q.is_empty() {
                let query = parse_text_query(
                    vec![ss.subject, ss.body_text, ss.from_address, ss.from_name],
                    q,
                )?;
                sub_queries.push((Occur::Must, query));
            }
        }

        if let Some(q) = from {
            if !q.is_empty() {
                let query = parse_text_query(vec![ss.from_address, ss.from_name], q)?;
                sub_queries.push((Occur::Must, query));
            }
        }

        if let Some(q) = to {
            if !q.is_empty() {
                let query = parse_text_query(vec![ss.to_addresses], q)?;
                sub_queries.push((Occur::Must, query));
            }
        }

        if let Some(q) = subject {
            if !q.is_empty() {
                let query = parse_text_query(vec![ss.subject], q)?;
                sub_queries.push((Occur::Must, query));
            }
        }

        // Date range filter
        if date_from.is_some() || date_to.is_some() {
            let lower = date_from
                .map(DateTime::from_timestamp_secs)
                .unwrap_or(DateTime::from_timestamp_secs(0));
            let upper = date_to
                .map(DateTime::from_timestamp_secs)
                .unwrap_or(DateTime::from_timestamp_secs(i64::MAX / 1_000_000)); // far future

            let range_query = RangeQuery::new_date_bounds(
                "date".to_string(),
                std::ops::Bound::Included(lower),
                std::ops::Bound::Included(upper),
            );
            sub_queries.push((Occur::Must, Box::new(range_query)));
        }

        // Has attachment filter
        if let Some(has_att) = has_attachment {
            let val = if has_att { "true" } else { "false" };
            let term = Term::from_field_text(ss.has_attachment, val);
            let term_query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
            sub_queries.push((Occur::Must, Box::new(term_query)));
        }

        // Folder filter
        if let Some(fid) = folder_id {
            if !fid.is_empty() {
                let term = Term::from_field_text(ss.folder_id, fid);
                let term_query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
                sub_queries.push((Occur::Must, Box::new(term_query)));
            }
        }

        // If no sub-queries, return empty
        if sub_queries.is_empty() {
            return Ok(Vec::new());
        }

        let bool_query = BooleanQuery::new(sub_queries);

        let top_docs = searcher
            .search(&bool_query, &TopDocs::with_limit(limit))
            .map_err(|e| PebbleError::Internal(format!("Search failed: {e}")))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_addr)
                .map_err(|e| PebbleError::Internal(format!("Failed to retrieve doc: {e}")))?;

            let message_id = doc
                .get_first(ss.message_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let snippet = make_snippet(&doc, ss.body_text);

            let subject = doc.get_first(ss.subject).and_then(|v| v.as_str()).map(|s| s.to_string());
            let from_address = doc.get_first(ss.from_address).and_then(|v| v.as_str()).map(|s| s.to_string());
            let date = doc.get_first(ss.date).and_then(|v| v.as_datetime()).map(|dt| dt.into_timestamp_secs());

            hits.push(SearchHit {
                message_id,
                score,
                snippet,
                subject,
                from_address,
                date,
            });
        }

        Ok(hits)
    }

    pub fn clear_index(&self) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| PebbleError::Internal(format!("Lock poisoned: {e}")))?;

        writer
            .delete_all_documents()
            .map_err(|e| PebbleError::Internal(format!("Failed to delete documents: {e}")))?;

        writer
            .commit()
            .map_err(|e| PebbleError::Internal(format!("Failed to commit after clear: {e}")))?;

        drop(writer); // release lock before reloading reader
        self.reader
            .reload()
            .map_err(|e| PebbleError::Internal(format!("Failed to reload reader: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::EmailAddress;

    fn make_test_message(id: &str, subject: &str, body: &str, from: &str) -> Message {
        Message {
            id: id.to_string(),
            account_id: "account-1".to_string(),
            remote_id: id.to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: subject.to_string(),
            snippet: subject.to_string(),
            from_address: from.to_string(),
            from_name: "Test Sender".to_string(),
            to_list: vec![EmailAddress {
                name: Some("Recipient".to_string()),
                address: "recipient@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: body.to_string(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 1_700_000_000,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn test_index_and_search_by_subject() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-1",
            "Invoice from Acme Corp",
            "Please find attached invoice.",
            "billing@acme.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("Invoice", 10).unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(hits[0].message_id, "msg-1");
    }

    #[test]
    fn test_search_by_body() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-2",
            "Meeting notes",
            "quarterly budget review discussion",
            "boss@company.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("quarterly budget", 10).unwrap();
        assert!(!hits.is_empty(), "expected body search to find the message");
        assert_eq!(hits[0].message_id, "msg-2");
    }

    #[test]
    fn test_search_by_from() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-3",
            "Hello there",
            "Just checking in.",
            "alice@wonderland.org",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("wonderland", 10).unwrap();
        assert!(!hits.is_empty(), "expected from_address search to find the message");
        assert_eq!(hits[0].message_id, "msg-3");
    }

    #[test]
    fn test_search_no_results() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-4",
            "Ordinary message",
            "Nothing special here.",
            "user@example.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("xyzzy_nonexistent_term_42", 10).unwrap();
        assert!(hits.is_empty(), "expected no results for nonexistent term");
    }

    #[test]
    fn test_clear_index() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-5",
            "Clearable message",
            "This will be deleted.",
            "temp@example.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        // Verify indexed
        let hits_before = engine.search("Clearable", 10).unwrap();
        assert!(!hits_before.is_empty(), "expected message before clear");

        engine.clear_index().unwrap();

        let hits_after = engine.search("Clearable", 10).unwrap();
        assert!(hits_after.is_empty(), "expected no results after clear");
    }

    #[test]
    fn test_reindex_same_message_replaces_old_document() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let mut msg = make_test_message(
            "msg-6",
            "Old subject",
            "old body",
            "sender@example.com",
        );

        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        msg.subject = "New subject".to_string();
        msg.body_text = "new body".to_string();
        engine.index_message(&msg, &["archive".to_string()]).unwrap();
        engine.commit().unwrap();

        let old_hits = engine.search("Old", 10).unwrap();
        assert!(old_hits.is_empty(), "expected old document to be replaced");

        let new_hits = engine.search("New", 10).unwrap();
        assert_eq!(new_hits.len(), 1, "expected one replacement document");

        let inbox_hits = engine
            .advanced_search(AdvancedSearchParams {
                text: Some("New"),
                from: None,
                to: None,
                subject: None,
                date_from: None,
                date_to: None,
                has_attachment: None,
                folder_id: Some("inbox"),
                limit: 10,
            })
            .unwrap();
        assert!(inbox_hits.is_empty(), "expected old folder mapping to be replaced");
    }

    #[test]
    fn test_search_cjk_chinese() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-cjk-1",
            "项目进度汇报",
            "本周已完成前端界面开发和后端接口对接",
            "zhangsan@example.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("前端界面", 10).unwrap();
        assert!(!hits.is_empty(), "expected CJK body search to find the message");
        assert_eq!(hits[0].message_id, "msg-cjk-1");

        let hits2 = engine.search("项目进度", 10).unwrap();
        assert!(!hits2.is_empty(), "expected CJK subject search to find the message");
    }

    #[test]
    fn test_snippet_shows_body_not_subject() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message(
            "msg-snippet",
            "Invoice from Acme",
            "Please find the quarterly financial report attached to this email.",
            "billing@acme.com",
        );
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine.search("quarterly", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(
            hits[0].snippet.contains("quarterly"),
            "snippet should contain body text, got: {}",
            hits[0].snippet
        );
        assert!(
            !hits[0].snippet.contains("Invoice"),
            "snippet should not be the subject"
        );
    }

    #[test]
    fn test_delete_by_account_removes_all_documents() {
        let engine = TantivySearch::open_in_memory().unwrap();

        // Index two messages for account-1 with unique subject/body terms
        let msg1 = make_test_message("msg-del-1", "Zephyr quarterly report", "zephyr financials here", "a@example.com");
        let msg2 = make_test_message("msg-del-2", "Zephyr project update", "zephyr milestone reached", "b@example.com");

        // Index one message for account-2 with different unique terms
        let mut msg3 = make_test_message("msg-del-3", "Pinnacle strategy memo", "pinnacle roadmap details", "c@example.com");
        msg3.account_id = "account-2".to_string();

        engine.index_message(&msg1, &["inbox".to_string()]).unwrap();
        engine.index_message(&msg2, &["inbox".to_string()]).unwrap();
        engine.index_message(&msg3, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        // Confirm all three are indexed
        assert_eq!(engine.doc_count(), 3);

        // Delete account-1's documents
        engine.delete_by_account("account-1").unwrap();

        // doc_count should drop to 1 (only account-2 message remains)
        assert_eq!(engine.doc_count(), 1, "expected two documents removed, one remaining");

        // account-1 messages should be gone — search for unique term "zephyr"
        let hits_a = engine.search("zephyr", 10).unwrap();
        assert!(hits_a.is_empty(), "expected account-1 messages to be removed from index");

        // account-2 message should still be present — search for unique term "pinnacle"
        let hits_c = engine.search("pinnacle", 10).unwrap();
        assert_eq!(hits_c.len(), 1, "expected account-2 message to remain in index");
        assert_eq!(hits_c[0].message_id, "msg-del-3");
    }

    #[test]
    fn test_search_finds_cc_recipients() {
        let engine = TantivySearch::open_in_memory().unwrap();
        let mut msg = make_test_message(
            "msg-cc",
            "Team update",
            "Weekly sync notes",
            "lead@company.com",
        );
        msg.cc_list = vec![EmailAddress {
            name: Some("Alice".to_string()),
            address: "alice@company.com".to_string(),
        }];
        engine.index_message(&msg, &["inbox".to_string()]).unwrap();
        engine.commit().unwrap();

        let hits = engine
            .advanced_search(AdvancedSearchParams {
                text: None,
                from: None,
                to: Some("alice"),
                subject: None,
                date_from: None,
                date_to: None,
                has_attachment: None,
                folder_id: None,
                limit: 10,
            })
            .unwrap();
        assert!(!hits.is_empty(), "expected CC recipient to be searchable via to filter");
        assert_eq!(hits[0].message_id, "msg-cc");
    }
}
