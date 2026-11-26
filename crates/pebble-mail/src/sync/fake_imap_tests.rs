//! Integration harness: drives `SyncWorker` against an in-process fake IMAP
//! server (tokio TCP, scripted responses) over a plain connection.

use super::*;
use crate::imap::{ConnectionSecurity, ImapConfig};
use pebble_core::{Account, ProviderType};
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// One message in the fake mailbox.
#[derive(Clone)]
struct FakeMessage {
    uid: u32,
    seen: bool,
    raw: Vec<u8>,
}

/// Mailbox state shared between the test and the server task.
#[derive(Default)]
struct FakeMailbox {
    uid_validity: u32,
    messages: Vec<FakeMessage>,
    /// Every command line received, without its tag (for assertions).
    commands: Vec<String>,
}

type Shared = Arc<StdMutex<FakeMailbox>>;

fn raw_message(uid: u32, subject: &str) -> Vec<u8> {
    format!(
        "Message-ID: <m{uid}@fake.test>\r\n\
         From: Sender <sender@fake.test>\r\n\
         To: me@fake.test\r\n\
         Subject: {subject}\r\n\
         Date: Mon, 1 Jan 2024 10:0{d}:00 +0000\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         Body of {subject}\r\n",
        d = uid % 10,
    )
    .into_bytes()
}

/// Parse a UID set such as `1:*`, `3,5:7` against the mailbox's UIDs.
fn uids_in_set(set: &str, all: &[u32]) -> Vec<u32> {
    let max = all.iter().copied().max().unwrap_or(0);
    let mut wanted = Vec::new();
    for part in set.split(',') {
        let (lo, hi) = match part.split_once(':') {
            Some((a, b)) => (a, b),
            None => (part, part),
        };
        let parse = |s: &str| {
            if s == "*" {
                max
            } else {
                s.parse::<u32>().unwrap_or(0)
            }
        };
        let (mut lo, mut hi) = (parse(lo), parse(hi));
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        wanted.extend(all.iter().copied().filter(|uid| *uid >= lo && *uid <= hi));
    }
    wanted.sort_unstable();
    wanted.dedup();
    wanted
}

fn flags(message: &FakeMessage) -> &'static str {
    if message.seen {
        "(\\Seen)"
    } else {
        "()"
    }
}

/// Untagged response bytes for one command (the tagged OK is appended by the caller).
fn respond(mailbox: &mut FakeMailbox, command: &str) -> Vec<u8> {
    let upper = command.to_ascii_uppercase();
    let mut out = Vec::new();
    let uids: Vec<u32> = mailbox.messages.iter().map(|m| m.uid).collect();
    let uid_next = uids.iter().copied().max().unwrap_or(0) + 1;

    if upper.starts_with("CAPABILITY") {
        push_line(&mut out, "* CAPABILITY IMAP4rev1 UIDPLUS MOVE");
    } else if upper.starts_with("LIST") {
        push_line(&mut out, "* LIST (\\HasNoChildren) \"/\" \"INBOX\"");
    } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
        push_line(&mut out, format!("* {} EXISTS", mailbox.messages.len()));
        push_line(&mut out, "* 0 RECENT");
        push_line(
            &mut out,
            format!("* OK [UIDVALIDITY {}] UIDs valid", mailbox.uid_validity),
        );
        push_line(
            &mut out,
            format!("* OK [UIDNEXT {uid_next}] Predicted next UID"),
        );
        push_line(&mut out, "* FLAGS (\\Seen \\Flagged \\Deleted)");
    } else if upper.starts_with("STATUS") {
        push_line(
            &mut out,
            format!(
                "* STATUS \"INBOX\" (MESSAGES {} UIDNEXT {uid_next} UIDVALIDITY {})",
                mailbox.messages.len(),
                mailbox.uid_validity
            ),
        );
    } else if upper.starts_with("UID SEARCH") {
        // Only "UID n:*"-style criteria are scripted; anything else returns all.
        let found = upper
            .split_whitespace()
            .skip(2)
            .find(|token| token.contains(':') || token.chars().all(|c| c.is_ascii_digit()))
            .map(|set| uids_in_set(set, &uids))
            .unwrap_or_else(|| uids.clone());
        let list: Vec<String> = found.iter().map(u32::to_string).collect();
        push_line(&mut out, format!("* SEARCH {}", list.join(" ")).trim_end());
    } else if upper.starts_with("UID FETCH") {
        let set = command.split_whitespace().nth(2).unwrap_or("");
        let wanted = uids_in_set(set, &uids);
        for (seq, message) in mailbox.messages.iter().enumerate() {
            if !wanted.contains(&message.uid) {
                continue;
            }
            let seq = seq + 1;
            if upper.contains("BODY.PEEK[]")
                || upper.contains("RFC822)")
                || upper.contains("BODY[]")
            {
                out.extend_from_slice(
                    format!(
                        "* {seq} FETCH (UID {} FLAGS {} RFC822.SIZE {} BODY[] {{{}}}\r\n",
                        message.uid,
                        flags(message),
                        message.raw.len(),
                        message.raw.len()
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(&message.raw);
                out.extend_from_slice(b")\r\n");
            } else if upper.contains("BODY.PEEK[HEADER]") {
                let header_end = message
                    .raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|i| i + 4)
                    .unwrap_or(message.raw.len());
                let header = &message.raw[..header_end];
                out.extend_from_slice(
                    format!(
                        "* {seq} FETCH (UID {} FLAGS {} BODY[HEADER] {{{}}}\r\n",
                        message.uid,
                        flags(message),
                        header.len()
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(header);
                out.extend_from_slice(b")\r\n");
            } else {
                push_line(
                    &mut out,
                    format!(
                        "* {seq} FETCH (UID {} FLAGS {} RFC822.SIZE {})",
                        message.uid,
                        flags(message),
                        message.raw.len()
                    ),
                );
            }
        }
    } else if upper.starts_with("FETCH") {
        // Sequence-number FETCH (UID/flags listings).
        for (seq, message) in mailbox.messages.iter().enumerate() {
            push_line(
                &mut out,
                format!(
                    "* {} FETCH (UID {} FLAGS {})",
                    seq + 1,
                    message.uid,
                    flags(message)
                ),
            );
        }
    }
    out
}

async fn serve(listener: TcpListener, mailbox: Shared) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let mailbox = Arc::clone(&mailbox);
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            if write
                .write_all(b"* OK [CAPABILITY IMAP4rev1] fake ready\r\n")
                .await
                .is_err()
            {
                return;
            }
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let line = buf.trim_end().to_string();
                // Swallow literals sent by the client (e.g. LOGIN {n}).
                if let Some(len) = line
                    .strip_suffix('}')
                    .and_then(|rest| rest.rsplit_once('{'))
                    .and_then(|(_, n)| n.trim_end_matches('+').parse::<usize>().ok())
                {
                    let _ = write.write_all(b"+ go ahead\r\n").await;
                    let mut literal = vec![0; len];
                    let _ = reader.read_exact(&mut literal).await;
                    let mut rest = String::new();
                    let _ = reader.read_line(&mut rest).await;
                }
                let Some((tag, command)) = line.split_once(' ') else {
                    continue;
                };
                let upper = command.to_ascii_uppercase();
                let response = {
                    let mut mailbox = mailbox.lock().unwrap();
                    mailbox.commands.push(command.to_string());
                    respond(&mut mailbox, command)
                };
                let _ = write.write_all(&response).await;
                if upper.starts_with("LOGOUT") {
                    let _ = write.write_all(b"* BYE logging out\r\n").await;
                    let _ = write
                        .write_all(format!("{tag} OK LOGOUT done\r\n").as_bytes())
                        .await;
                    return;
                }
                let _ = write
                    .write_all(
                        format!(
                            "{tag} OK {} done\r\n",
                            upper.split(' ').next().unwrap_or("")
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
    }
}

async fn start_fake_imap(mailbox: Shared) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve(listener, mailbox));
    port
}

fn harness_store() -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let now = now_timestamp();
    store
        .insert_account(&Account {
            id: "acct".to_string(),
            email: "me@fake.test".to_string(),
            display_name: "Me".to_string(),
            account_label: None,
            provider_display_name: None,
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    store
}

fn harness_worker(port: u16, store: Arc<Store>, dir: &Path) -> SyncWorker {
    let provider = Arc::new(ImapMailProvider::new(ImapConfig {
        host: "127.0.0.1".to_string(),
        port,
        username: "me".to_string(),
        password: "secret".to_string(),
        security: ConnectionSecurity::Plain,
        accept_invalid_certs: false,
        proxy: None,
    }));
    let (_stop_tx, stop_rx) = watch::channel(false);
    SyncWorker::new("acct", provider, store, stop_rx, dir)
}

fn inbox_subjects(store: &Store) -> Vec<String> {
    let inbox = store
        .list_folders("acct")
        .unwrap()
        .into_iter()
        .find(|folder| folder.role == Some(pebble_core::FolderRole::Inbox))
        .expect("inbox folder");
    let mut subjects: Vec<String> = store
        .list_messages_by_folder(&inbox.id, 100, 0)
        .unwrap()
        .into_iter()
        .map(|message| message.subject)
        .collect();
    subjects.sort();
    subjects
}

#[tokio::test]
async fn backfill_page_then_reconcile_against_fake_imap_server() {
    let mailbox: Shared = Arc::new(StdMutex::new(FakeMailbox {
        uid_validity: 7,
        messages: (1..=3)
            .map(|uid| FakeMessage {
                uid,
                seen: uid == 1,
                raw: raw_message(uid, &format!("Message {uid}")),
            })
            .collect(),
        commands: Vec::new(),
    }));
    let port = start_fake_imap(Arc::clone(&mailbox)).await;
    let store = harness_store();
    let dir = tempfile_dir();
    let worker = harness_worker(port, Arc::clone(&store), &dir);

    worker.provider.connect().await.unwrap();
    worker.initial_sync().await.unwrap();

    assert_eq!(
        inbox_subjects(&store),
        vec!["Message 1", "Message 2", "Message 3"],
        "commands: {:#?}",
        mailbox.lock().unwrap().commands
    );

    let inbox_messages = |store: &Store| {
        let inbox = store
            .list_folders("acct")
            .unwrap()
            .into_iter()
            .find(|folder| folder.role == Some(pebble_core::FolderRole::Inbox))
            .unwrap();
        store.list_messages_by_folder(&inbox.id, 100, 0).unwrap()
    };
    // Backfill also pulls the server's flags for new messages, so message 1
    // (\Seen on the server) starts read without waiting for a reconcile.
    let initial = inbox_messages(&store);
    assert_eq!(initial.len(), 3);
    for message in &initial {
        assert_eq!(
            message.is_read,
            message.subject == "Message 1",
            "initial read state of {}: {:#?}",
            message.subject,
            mailbox.lock().unwrap().commands
        );
    }
    {
        let commands = &mailbox.lock().unwrap().commands;
        assert!(commands.iter().any(|c| c.starts_with("LOGIN")));
        assert!(commands.iter().any(|c| c.contains("BODY.PEEK[]")));
    }

    // Server side: message 2 is expunged and message 3 is read elsewhere.
    {
        let mut mailbox = mailbox.lock().unwrap();
        mailbox.messages.retain(|message| message.uid != 2);
        mailbox
            .messages
            .iter_mut()
            .filter(|message| message.uid == 3)
            .for_each(|message| message.seen = true);
    }

    let inbox = store
        .list_folders("acct")
        .unwrap()
        .into_iter()
        .find(|folder| folder.role == Some(pebble_core::FolderRole::Inbox))
        .unwrap();
    // Flag reconcile skips rows touched in the last minute (local write-back
    // race guard); age the freshly synced rows past it.
    store
        .with_write_async(|conn| {
            conn.execute("UPDATE messages SET updated_at = updated_at - 3600", [])?;
            Ok(())
        })
        .await
        .unwrap();
    worker.reconcile_folder(&inbox).await.unwrap();

    assert_eq!(
        inbox_subjects(&store),
        vec!["Message 1", "Message 3"],
        "commands: {:#?}",
        mailbox.lock().unwrap().commands
    );
    // Both messages are \Seen on the server now; reconcile pulls the flags.
    let after = inbox_messages(&store);
    assert!(
        after.iter().all(|message| message.is_read),
        "reconcile must pull the remote \\Seen flags: {:#?}",
        mailbox.lock().unwrap().commands
    );
    assert!(mailbox
        .lock()
        .unwrap()
        .commands
        .iter()
        .any(|c| c.starts_with("UID FETCH") && c.contains("FLAGS")));
    let _ = worker.provider.disconnect().await;
    let _ = std::fs::remove_dir_all(dir);
}

fn tempfile_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pebble-fake-imap-{}", new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn push_line(out: &mut Vec<u8>, text: impl AsRef<str>) {
    out.extend_from_slice(text.as_ref().as_bytes());
    out.extend_from_slice(b"\r\n");
}
