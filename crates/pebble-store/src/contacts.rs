use std::collections::{HashMap, HashSet};

use pebble_core::{
    Contact, ContactEmail, ContactEmailInput, ContactEmailLabel, ContactInput, ContactSuggestion,
    ContactSuggestionSource, KnownContact, PebbleError, Result,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::Store;

pub(crate) const MAX_CONTACT_DISPLAY_NAME_CHARS: usize = 512;

fn contact_label_to_str(label: &ContactEmailLabel) -> &'static str {
    match label {
        ContactEmailLabel::Work => "work",
        ContactEmailLabel::Personal => "personal",
        ContactEmailLabel::Other => "other",
    }
}

fn str_to_contact_label(label: &str) -> ContactEmailLabel {
    match label {
        "work" => ContactEmailLabel::Work,
        "personal" => ContactEmailLabel::Personal,
        _ => ContactEmailLabel::Other,
    }
}

fn prepare_email(input: &ContactEmailInput) -> Result<(String, String)> {
    let address = input.address.trim().to_string();
    if address.is_empty() || address.len() > 320 {
        return Err(PebbleError::Validation(
            "Email address is required and must not exceed 320 characters".to_string(),
        ));
    }
    if address.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(PebbleError::Validation(format!(
            "Invalid email address: {}",
            input.address
        )));
    }
    let Some((local, domain)) = address.split_once('@') else {
        return Err(PebbleError::Validation(format!(
            "Invalid email address: {}",
            input.address
        )));
    };
    if local.is_empty()
        || local.len() > 64
        || domain.is_empty()
        || domain.contains('@')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !domain.contains('.')
    {
        return Err(PebbleError::Validation(format!(
            "Invalid email address: {}",
            input.address
        )));
    }
    Ok((address.clone(), address.to_lowercase()))
}

fn validate_contact_input(input: &ContactInput) -> Result<Vec<(String, String)>> {
    if input.display_name.chars().count() > MAX_CONTACT_DISPLAY_NAME_CHARS {
        return Err(PebbleError::Validation(format!(
            "Contact display name must not exceed {MAX_CONTACT_DISPLAY_NAME_CHARS} characters"
        )));
    }
    if input.emails.is_empty() {
        return Err(PebbleError::Validation(
            "A contact must have at least one email address".to_string(),
        ));
    }
    if input.notes.chars().count() > 2000 {
        return Err(PebbleError::Validation(
            "Contact notes must not exceed 2000 characters".to_string(),
        ));
    }
    let primary_count = input.emails.iter().filter(|email| email.is_primary).count();
    if primary_count != 1 {
        return Err(PebbleError::Validation(
            "A contact must have exactly one primary email address".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    let mut prepared = Vec::with_capacity(input.emails.len());
    for email in &input.emails {
        let values = prepare_email(email)?;
        if !seen.insert(values.1.clone()) {
            return Err(PebbleError::Validation(format!(
                "Duplicate email address: {}",
                email.address.trim()
            )));
        }
        prepared.push(values);
    }
    Ok(prepared)
}

fn map_contact_email_insert_error(error: rusqlite::Error, address: &str) -> PebbleError {
    if let rusqlite::Error::SqliteFailure(sqlite_error, _) = &error {
        if matches!(
            sqlite_error.extended_code,
            rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY | rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
        ) {
            return PebbleError::Validation(format!(
                "Unable to save contact email {address}: {error}"
            ));
        }
    }
    PebbleError::from(error)
}

pub(crate) fn load_contact_with_conn(
    conn: &Connection,
    contact_id: &str,
) -> Result<Option<Contact>> {
    let row = conn
        .query_row(
            "SELECT id, display_name, notes, is_favorite, created_at, updated_at
             FROM contacts WHERE id = ?1",
            params![contact_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((id, display_name, notes, is_favorite, created_at, updated_at)) = row else {
        return Ok(None);
    };

    let mut stmt = conn.prepare(
        "SELECT id, address, label, is_primary
         FROM contact_emails
         WHERE contact_id = ?1
         ORDER BY is_primary DESC, created_at ASC, id ASC",
    )?;
    let emails = stmt
        .query_map(params![id], |row| {
            Ok(ContactEmail {
                id: row.get(0)?,
                address: row.get(1)?,
                label: str_to_contact_label(&row.get::<_, String>(2)?),
                is_primary: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(Some(Contact {
        id,
        display_name,
        notes,
        is_favorite,
        emails,
        created_at,
        updated_at,
    }))
}

#[derive(Debug)]
struct RecentContactCandidate {
    name: Option<String>,
    address: String,
    last_interaction_at: i64,
}

fn add_recent_candidate(
    candidates: &mut HashMap<String, RecentContactCandidate>,
    self_address: &str,
    name: Option<String>,
    address: String,
    date: i64,
) {
    let trimmed = address.trim();
    let normalized = trimmed.to_lowercase();
    if normalized.is_empty() || normalized == self_address {
        return;
    }
    let email = ContactEmailInput {
        id: None,
        address: trimmed.to_string(),
        label: ContactEmailLabel::Other,
        is_primary: true,
    };
    if prepare_email(&email).is_err() {
        return;
    }
    let name = name.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    });

    match candidates.get_mut(&normalized) {
        Some(existing) if date > existing.last_interaction_at => {
            *existing = RecentContactCandidate {
                name,
                address: trimmed.to_string(),
                last_interaction_at: date,
            };
        }
        Some(existing) if existing.name.is_none() && name.is_some() => {
            existing.name = name;
        }
        Some(_) => {}
        None => {
            candidates.insert(
                normalized,
                RecentContactCandidate {
                    name,
                    address: trimmed.to_string(),
                    last_interaction_at: date,
                },
            );
        }
    }
}

pub(crate) fn save_contact_with_conn(conn: &Connection, input: &ContactInput) -> Result<Contact> {
    let prepared_emails = validate_contact_input(input)?;
    let display_name = input.display_name.trim().to_string();
    let notes = input.notes.trim().to_string();
    let now = pebble_core::now_timestamp();
    let (contact_id, created_at, existing_email_ids) = if let Some(id) = &input.id {
        if id.trim().is_empty() {
            return Err(PebbleError::Validation(
                "Contact id must not be empty".to_string(),
            ));
        }
        let created_at = conn
            .query_row(
                "SELECT created_at FROM contacts WHERE id = ?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .ok_or_else(|| PebbleError::Validation(format!("Contact not found: {id}")))?;
        let mut stmt = conn.prepare("SELECT id FROM contact_emails WHERE contact_id = ?1")?;
        let existing_ids = stmt
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        (id.clone(), created_at, existing_ids)
    } else {
        (pebble_core::new_id(), now, HashSet::new())
    };

    for (_, normalized) in &prepared_emails {
        let owner: Option<String> = conn
            .query_row(
                "SELECT contact_id FROM contact_emails
                 WHERE normalized_address = ?1 COLLATE NOCASE AND contact_id != ?2",
                params![normalized, contact_id],
                |row| row.get(0),
            )
            .optional()?;
        if owner.is_some() {
            return Err(PebbleError::Validation(format!(
                "Email address already belongs to another contact: {normalized}"
            )));
        }
    }

    if input.id.is_some() {
        conn.execute(
            "UPDATE contacts
             SET display_name = ?1, notes = ?2, is_favorite = ?3, updated_at = ?4
             WHERE id = ?5",
            params![display_name, notes, input.is_favorite, now, contact_id],
        )?;
        conn.execute(
            "DELETE FROM contact_emails WHERE contact_id = ?1",
            params![contact_id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO contacts
                (id, display_name, notes, is_favorite, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                contact_id,
                display_name,
                notes,
                input.is_favorite,
                created_at,
                now
            ],
        )?;
    }

    for (index, email) in input.emails.iter().enumerate() {
        let (address, normalized) = &prepared_emails[index];
        let email_id = email
            .id
            .as_ref()
            .filter(|id| existing_email_ids.contains(*id))
            .cloned()
            .unwrap_or_else(pebble_core::new_id);
        conn.execute(
            "INSERT INTO contact_emails
                (id, contact_id, address, normalized_address, label, is_primary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                email_id,
                contact_id,
                address,
                normalized,
                contact_label_to_str(&email.label),
                email.is_primary,
                now
            ],
        )
        .map_err(|error| map_contact_email_insert_error(error, address))?;
    }

    load_contact_with_conn(conn, &contact_id)?
        .ok_or_else(|| PebbleError::Internal("Saved contact could not be loaded".to_string()))
}

pub(crate) fn replace_contacts_with_conn(
    conn: &Transaction<'_>,
    contacts: &[Contact],
) -> Result<()> {
    let mut contact_ids = HashSet::new();
    let mut email_ids = HashSet::new();
    let mut normalized_addresses = HashSet::new();
    let mut prepared_contacts = Vec::with_capacity(contacts.len());
    for contact in contacts {
        if contact.id.trim().is_empty() {
            return Err(PebbleError::Validation(
                "Restored contact id must not be empty".to_string(),
            ));
        }
        if !contact_ids.insert(contact.id.clone()) {
            return Err(PebbleError::Validation(format!(
                "Duplicate restored contact id: {}",
                contact.id
            )));
        }

        let input = ContactInput {
            id: Some(contact.id.clone()),
            display_name: contact.display_name.clone(),
            notes: contact.notes.clone(),
            is_favorite: contact.is_favorite,
            emails: contact
                .emails
                .iter()
                .map(|email| ContactEmailInput {
                    id: Some(email.id.clone()),
                    address: email.address.clone(),
                    label: email.label.clone(),
                    is_primary: email.is_primary,
                })
                .collect(),
        };
        let prepared_emails = validate_contact_input(&input)?;

        for (email, (_, normalized)) in contact.emails.iter().zip(&prepared_emails) {
            if email.id.trim().is_empty() {
                return Err(PebbleError::Validation(
                    "Restored contact email id must not be empty".to_string(),
                ));
            }
            if !email_ids.insert(email.id.clone()) {
                return Err(PebbleError::Validation(format!(
                    "Duplicate restored contact email id: {}",
                    email.id
                )));
            }
            if !normalized_addresses.insert(normalized.clone()) {
                return Err(PebbleError::Validation(format!(
                    "Duplicate restored contact email address: {normalized}"
                )));
            }
        }
        prepared_contacts.push((contact, prepared_emails));
    }

    conn.execute("DELETE FROM contacts", [])?;

    for (contact, prepared_emails) in prepared_contacts {
        conn.execute(
            "INSERT INTO contacts
                (id, display_name, notes, is_favorite, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                contact.id,
                contact.display_name.trim(),
                contact.notes.trim(),
                contact.is_favorite,
                contact.created_at,
                contact.updated_at
            ],
        )?;

        for (email, (address, normalized)) in contact.emails.iter().zip(prepared_emails) {
            conn.execute(
                "INSERT INTO contact_emails
                    (id, contact_id, address, normalized_address, label, is_primary, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    email.id,
                    contact.id,
                    address,
                    normalized,
                    contact_label_to_str(&email.label),
                    email.is_primary,
                    contact.created_at
                ],
            )?;
        }
    }

    Ok(())
}

pub(crate) fn list_contacts_with_conn(
    conn: &Connection,
    query: Option<&str>,
    favorite_only: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<Contact>> {
    let query = query.unwrap_or_default().trim();
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let mut stmt = conn.prepare(
        "SELECT c.id
         FROM contacts c
         WHERE (?1 = '' OR c.display_name LIKE ?2 ESCAPE '\\' COLLATE NOCASE
                OR EXISTS (
                    SELECT 1 FROM contact_emails ce
                    WHERE ce.contact_id = c.id
                      AND ce.address LIKE ?2 ESCAPE '\\' COLLATE NOCASE
                ))
           AND (?3 = 0 OR c.is_favorite = 1)
         ORDER BY LOWER(CASE
             WHEN c.display_name = '' THEN COALESCE((
                 SELECT ce.address FROM contact_emails ce
                 WHERE ce.contact_id = c.id
                 ORDER BY ce.is_primary DESC, ce.created_at ASC LIMIT 1
             ), '')
             ELSE c.display_name
         END) ASC, c.id ASC
         LIMIT ?4 OFFSET ?5",
    )?;
    let ids = stmt
        .query_map(
            params![query, pattern, favorite_only, limit, offset],
            |row| row.get::<_, String>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    ids.into_iter()
        .map(|id| {
            load_contact_with_conn(conn, &id)?.ok_or_else(|| {
                PebbleError::Internal(format!("Contact disappeared while listing: {id}"))
            })
        })
        .collect()
}

pub(crate) fn list_all_contacts_for_backup_with_conn(
    conn: &Transaction<'_>,
) -> Result<Vec<Contact>> {
    let mut contacts = Vec::new();
    loop {
        let page = list_contacts_with_conn(conn, None, false, 200, contacts.len() as i64)?;
        let page_len = page.len();
        contacts.extend(page);
        if page_len < 200 {
            return Ok(contacts);
        }
    }
}

impl Store {
    pub fn save_contact(&self, input: &ContactInput) -> Result<Contact> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let contact = save_contact_with_conn(&tx, input)?;
            tx.commit()?;
            Ok(contact)
        })
    }

    pub fn get_contact(&self, contact_id: &str) -> Result<Option<Contact>> {
        self.with_read(|conn| load_contact_with_conn(conn, contact_id))
    }

    pub fn get_contact_by_email(&self, address: &str) -> Result<Option<Contact>> {
        let (_, normalized) = prepare_email(&ContactEmailInput {
            id: None,
            address: address.to_string(),
            label: ContactEmailLabel::Other,
            is_primary: true,
        })?;
        self.with_read(|conn| {
            let contact_id = conn
                .query_row(
                    "SELECT contact_id
                     FROM contact_emails
                     WHERE normalized_address = ?1 COLLATE NOCASE",
                    params![normalized],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match contact_id {
                Some(contact_id) => load_contact_with_conn(conn, &contact_id),
                None => Ok(None),
            }
        })
    }

    pub fn list_contacts(
        &self,
        query: Option<&str>,
        favorite_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Contact>> {
        self.with_read(|conn| list_contacts_with_conn(conn, query, favorite_only, limit, offset))
    }

    pub fn delete_contact(&self, contact_id: &str, suppress_addresses: bool) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            if suppress_addresses {
                let addresses = {
                    let mut stmt = tx.prepare(
                        "SELECT normalized_address FROM contact_emails WHERE contact_id = ?1",
                    )?;
                    let values = stmt
                        .query_map(params![contact_id], |row| row.get::<_, String>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    values
                };
                let now = pebble_core::now_timestamp();
                for address in addresses {
                    tx.execute(
                        "INSERT OR IGNORE INTO contact_suggestion_suppressions
                            (normalized_address, created_at) VALUES (?1, ?2)",
                        params![address, now],
                    )?;
                }
            }
            let deleted = tx.execute("DELETE FROM contacts WHERE id = ?1", params![contact_id])?;
            if deleted == 0 {
                return Err(PebbleError::Validation(format!(
                    "Contact not found: {contact_id}"
                )));
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn set_contact_favorite(&self, contact_id: &str, is_favorite: bool) -> Result<()> {
        self.with_write(|conn| {
            let updated = conn.execute(
                "UPDATE contacts SET is_favorite = ?1, updated_at = ?2 WHERE id = ?3",
                params![is_favorite, pebble_core::now_timestamp(), contact_id],
            )?;
            if updated == 0 {
                return Err(PebbleError::Validation(format!(
                    "Contact not found: {contact_id}"
                )));
            }
            Ok(())
        })
    }

    pub fn suppress_contact_suggestion(&self, address: &str) -> Result<()> {
        let (_, normalized) = prepare_email(&ContactEmailInput {
            id: None,
            address: address.to_string(),
            label: ContactEmailLabel::Other,
            is_primary: true,
        })?;
        self.with_write(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO contact_suggestion_suppressions
                    (normalized_address, created_at) VALUES (?1, ?2)",
                params![normalized, pebble_core::now_timestamp()],
            )?;
            Ok(())
        })
    }

    pub fn search_contact_suggestions(
        &self,
        account_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ContactSuggestion>> {
        let limit = limit.clamp(1, 100) as usize;
        let candidate_limit = (limit.saturating_mul(5)).max(100) as i64;
        let query = query.trim();
        let lower_query = query.to_lowercase();
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");

        self.with_read(|conn| {
            let self_address = conn
                .query_row(
                    "SELECT email FROM accounts WHERE id = ?1",
                    params![account_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .ok_or_else(|| {
                    PebbleError::Validation(format!("Account not found: {account_id}"))
                })?
                .trim()
                .to_lowercase();

            let suppressed = {
                let mut stmt = conn.prepare(
                    "SELECT normalized_address FROM contact_suggestion_suppressions",
                )?;
                let values = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<HashSet<_>, _>>()?;
                values
            };

            // Recent correspondents come from the derived, per-account
            // `message_correspondents` table (maintained on message writes)
            // instead of a `%x%` LIKE scan over every message's recipient
            // JSON on each keystroke.
            let mut recent = HashMap::new();
            let mut history_stmt = conn.prepare_cached(
                "SELECT name, address, last_interaction_at
                 FROM message_correspondents
                 WHERE account_id = ?1
                   AND (?2 = ''
                        OR normalized_address LIKE ?3 ESCAPE '\\' COLLATE NOCASE
                        OR name LIKE ?3 ESCAPE '\\' COLLATE NOCASE)
                 ORDER BY last_interaction_at DESC
                 LIMIT 1000",
            )?;
            let history_rows = history_stmt.query_map(params![account_id, query, pattern], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            for row in history_rows {
                let (name, address, date) = row?;
                add_recent_candidate(&mut recent, &self_address, name, address, date);
            }
            drop(history_stmt);

            let mut suggestions = Vec::new();
            let mut seen = HashSet::new();
            let mut saved_stmt = conn.prepare(
                "SELECT c.id, c.display_name, c.is_favorite,
                        ce.address, ce.normalized_address
                 FROM contacts c
                 JOIN contact_emails ce ON ce.contact_id = c.id
                 WHERE (?1 = '' OR c.display_name LIKE ?2 ESCAPE '\\' COLLATE NOCASE
                        OR ce.address LIKE ?2 ESCAPE '\\' COLLATE NOCASE)
                 ORDER BY c.is_favorite DESC,
                          LOWER(CASE WHEN c.display_name = '' THEN ce.address ELSE c.display_name END),
                          ce.is_primary DESC,
                          LOWER(ce.address), ce.id
                 LIMIT ?3",
            )?;
            let saved_rows = saved_stmt.query_map(
                params![query, pattern, candidate_limit],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
            for row in saved_rows {
                let (contact_id, display_name, is_favorite, address, normalized) = row?;
                let normalized = normalized.to_lowercase();
                if normalized == self_address || !seen.insert(normalized.clone()) {
                    continue;
                }
                suggestions.push(ContactSuggestion {
                    contact_id: Some(contact_id),
                    name: (!display_name.trim().is_empty()).then_some(display_name),
                    address,
                    source: ContactSuggestionSource::Saved,
                    is_favorite,
                    last_interaction_at: recent
                        .get(&normalized)
                        .map(|item| item.last_interaction_at),
                });
            }
            drop(saved_stmt);

            let mut recent_entries = recent.into_iter().collect::<Vec<_>>();
            recent_entries.sort_by(|(left_address, left), (right_address, right)| {
                right
                    .last_interaction_at
                    .cmp(&left.last_interaction_at)
                    .then_with(|| left_address.cmp(right_address))
            });
            for (normalized, candidate) in recent_entries {
                if suggestions.len() >= limit {
                    break;
                }
                let name_matches = candidate
                    .name
                    .as_ref()
                    .map(|name| name.to_lowercase().contains(&lower_query))
                    .unwrap_or(false);
                if (!lower_query.is_empty()
                    && !normalized.contains(&lower_query)
                    && !name_matches)
                    || suppressed.contains(&normalized)
                    || !seen.insert(normalized)
                {
                    continue;
                }
                suggestions.push(ContactSuggestion {
                    contact_id: None,
                    name: candidate.name,
                    address: candidate.address,
                    source: ContactSuggestionSource::Recent,
                    is_favorite: false,
                    last_interaction_at: Some(candidate.last_interaction_at),
                });
            }

            suggestions.truncate(limit);
            Ok(suggestions)
        })
    }

    /// Known correspondents of an account whose address or name contains
    /// `query`, most recent first. Backed by `message_correspondents`
    /// (sender plus To/Cc/Bcc of every stored live message), deduplicated by
    /// address (case-insensitive) and limited to `limit` rows.
    pub fn list_known_contacts(
        &self,
        account_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<KnownContact>> {
        self.with_read(|conn| {
            let escaped = query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{}%", escaped);

            let mut stmt = conn.prepare_cached(
                "SELECT name, address
                 FROM message_correspondents
                 WHERE account_id = ?1
                   AND (normalized_address LIKE ?2 ESCAPE '\\' COLLATE NOCASE
                        OR name LIKE ?2 ESCAPE '\\' COLLATE NOCASE)
                 ORDER BY last_interaction_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![account_id, pattern, limit], |row| {
                Ok(KnownContact {
                    name: row
                        .get::<_, Option<String>>(0)?
                        .filter(|name| !name.is_empty()),
                    address: row.get(1)?,
                })
            })?;
            let contacts = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(contacts)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{list_all_contacts_for_backup_with_conn, replace_contacts_with_conn};
    use crate::Store;
    use pebble_core::*;

    fn contact_input(name: &str, address: &str) -> ContactInput {
        ContactInput {
            id: None,
            display_name: name.to_string(),
            notes: String::new(),
            is_favorite: false,
            emails: vec![ContactEmailInput {
                id: None,
                address: address.to_string(),
                label: ContactEmailLabel::Other,
                is_primary: true,
            }],
        }
    }

    fn setup_store_with_contacts() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let account = Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "me@example.com".to_string(),
            display_name: "Me".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();

        let folder = Folder {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();

        // Message from alice
        let msg1 = Message {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "1".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Hello".to_string(),
            snippet: "hi".to_string(),
            from_address: "alice@example.com".to_string(),
            from_name: "Alice Smith".to_string(),
            to_list: vec![EmailAddress {
                name: Some("Bob Jones".to_string()),
                address: "bob@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "hi".to_string(),
            body_html_raw: "<p>hi</p>".to_string(),
            has_attachments: false,
            is_read: true,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };
        store
            .insert_message(&msg1, std::slice::from_ref(&folder.id))
            .unwrap();

        // Message from charlie
        let msg2 = Message {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "2".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Hey".to_string(),
            snippet: "hey".to_string(),
            from_address: "charlie@other.com".to_string(),
            from_name: "Charlie".to_string(),
            to_list: vec![EmailAddress {
                name: Some("Me".to_string()),
                address: "me@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "hey".to_string(),
            body_html_raw: "<p>hey</p>".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };
        store
            .insert_message(&msg2, std::slice::from_ref(&folder.id))
            .unwrap();

        (store, account.id)
    }

    fn setup_suggestion_store() -> (Store, String, String) {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let account = Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "me@example.com".to_string(),
            display_name: "Me".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = Folder {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        (store, account.id, folder.id)
    }

    struct SuggestionMessage<'a> {
        remote_id: &'a str,
        from_name: &'a str,
        from_address: &'a str,
        to: Vec<EmailAddress>,
        cc: Vec<EmailAddress>,
        bcc: Vec<EmailAddress>,
        date: i64,
    }

    fn insert_suggestion_message(
        store: &Store,
        account_id: &str,
        folder_id: &str,
        message: SuggestionMessage<'_>,
    ) {
        let saved = Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: message.remote_id.to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Contact history".to_string(),
            snippet: String::new(),
            from_address: message.from_address.to_string(),
            from_name: message.from_name.to_string(),
            to_list: message.to,
            cc_list: message.cc,
            bcc_list: message.bcc,
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: true,
            is_starred: false,
            is_draft: false,
            date: message.date,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: message.date,
            updated_at: message.date,
        };
        store
            .insert_message(&saved, &[folder_id.to_string()])
            .unwrap();
    }

    #[test]
    fn test_list_known_contacts_by_from_address() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store.list_known_contacts(&account_id, "alice", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].address, "alice@example.com");
        assert_eq!(results[0].name.as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn test_list_known_contacts_by_to_list() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store.list_known_contacts(&account_id, "bob", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].address, "bob@example.com");
        assert_eq!(results[0].name.as_deref(), Some("Bob Jones"));
    }

    #[test]
    fn test_list_known_contacts_by_cc_and_bcc_lists() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "copy-recipients",
                from_name: "Sender",
                from_address: "sender@example.com",
                to: vec![],
                cc: vec![EmailAddress {
                    name: Some("Copy Person".to_string()),
                    address: "copy@example.com".to_string(),
                }],
                bcc: vec![EmailAddress {
                    name: Some("Blind Person".to_string()),
                    address: "blind@example.com".to_string(),
                }],
                date: 100,
            },
        );

        let cc = store.list_known_contacts(&account_id, "copy", 10).unwrap();
        let bcc = store.list_known_contacts(&account_id, "blind", 10).unwrap();

        assert_eq!(cc[0].address, "copy@example.com");
        assert_eq!(bcc[0].address, "blind@example.com");
    }

    #[test]
    fn test_list_known_contacts_broad_query() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store
            .list_known_contacts(&account_id, "example", 10)
            .unwrap();
        // alice@example.com from from_address, bob@example.com from to_list, me@example.com from to_list
        assert!(results.len() >= 2);
    }

    #[test]
    fn test_list_known_contacts_empty_query() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store.list_known_contacts(&account_id, "", 10).unwrap();
        // Should return all known contacts
        assert!(results.len() >= 2);
    }

    #[test]
    fn test_list_known_contacts_respects_limit() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store.list_known_contacts(&account_id, "", 1).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_list_known_contacts_no_match() {
        let (store, account_id) = setup_store_with_contacts();
        let results = store
            .list_known_contacts(&account_id, "zzzznotfound", 10)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn contact_crud_round_trips_multiple_emails() {
        let store = Store::open_in_memory().unwrap();
        let input = ContactInput {
            id: None,
            display_name: " Alice Smith ".to_string(),
            notes: "Met at RustConf".to_string(),
            is_favorite: true,
            emails: vec![
                ContactEmailInput {
                    id: None,
                    address: " Alice@Example.com ".to_string(),
                    label: ContactEmailLabel::Work,
                    is_primary: true,
                },
                ContactEmailInput {
                    id: None,
                    address: "alice@home.example".to_string(),
                    label: ContactEmailLabel::Personal,
                    is_primary: false,
                },
            ],
        };

        let saved = store.save_contact(&input).unwrap();
        assert_eq!(saved.display_name, "Alice Smith");
        assert_eq!(saved.emails.len(), 2);
        assert_eq!(saved.emails[0].address, "Alice@Example.com");
        assert!(saved.emails[0].is_primary);
        assert!(saved.is_favorite);

        let loaded = store.get_contact(&saved.id).unwrap().unwrap();
        assert_eq!(loaded, saved);
    }

    #[test]
    fn contact_lookup_by_email_is_exact_and_case_insensitive() {
        let store = Store::open_in_memory().unwrap();
        let alice = store
            .save_contact(&contact_input("Alice", "Alice@Example.com"))
            .unwrap();
        store
            .save_contact(&contact_input("Alias", "alice+other@example.com"))
            .unwrap();

        let found = store
            .get_contact_by_email("  alice@EXAMPLE.COM ")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, alice.id);
        assert!(store
            .get_contact_by_email("missing@example.com")
            .unwrap()
            .is_none());
    }

    #[test]
    fn contact_save_requires_email() {
        let store = Store::open_in_memory().unwrap();
        let no_email = ContactInput {
            emails: vec![],
            ..contact_input("Nobody", "unused@example.com")
        };
        assert!(matches!(
            store.save_contact(&no_email),
            Err(PebbleError::Validation(_))
        ));
    }

    #[test]
    fn contact_save_rejects_invalid_email() {
        let store = Store::open_in_memory().unwrap();
        let invalid_email = contact_input("Invalid", "not-an-address");
        assert!(matches!(
            store.save_contact(&invalid_email),
            Err(PebbleError::Validation(_))
        ));
    }

    #[test]
    fn contact_save_requires_exactly_one_primary_email() {
        let store = Store::open_in_memory().unwrap();
        let multiple_primary = ContactInput {
            emails: vec![
                ContactEmailInput {
                    id: None,
                    address: "one@example.com".to_string(),
                    label: ContactEmailLabel::Work,
                    is_primary: true,
                },
                ContactEmailInput {
                    id: None,
                    address: "two@example.com".to_string(),
                    label: ContactEmailLabel::Personal,
                    is_primary: true,
                },
            ],
            ..contact_input("Two Primaries", "unused@example.com")
        };
        assert!(matches!(
            store.save_contact(&multiple_primary),
            Err(PebbleError::Validation(_))
        ));
    }

    #[test]
    fn contact_save_rejects_notes_over_limit() {
        let store = Store::open_in_memory().unwrap();
        let input = ContactInput {
            notes: "a".repeat(2001),
            ..contact_input("Verbose", "verbose@example.com")
        };

        assert!(matches!(
            store.save_contact(&input),
            Err(PebbleError::Validation(_))
        ));
    }

    #[test]
    fn contact_save_rejects_display_name_over_limit() {
        let store = Store::open_in_memory().unwrap();
        let input = ContactInput {
            display_name: "a".repeat(513),
            ..contact_input("unused", "long-name@example.com")
        };

        assert!(matches!(
            store.save_contact(&input),
            Err(PebbleError::Validation(message)) if message.contains("512")
        ));
    }

    #[test]
    fn contact_email_operational_errors_remain_storage_errors() {
        let store = Store::open_in_memory().unwrap();
        store
            .with_write(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER fail_contact_email_insert
                     BEFORE INSERT ON contact_emails
                     BEGIN
                         SELECT RAISE(FAIL, 'simulated contact email storage failure');
                     END;",
                )?;
                Ok(())
            })
            .unwrap();

        assert!(matches!(
            store.save_contact(&contact_input("Alice", "alice@example.com")),
            Err(PebbleError::Storage(message))
                if message.contains("simulated contact email storage failure")
        ));
    }

    #[test]
    fn contact_restore_validates_before_deleting_existing_contacts() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_contact(&contact_input("Keep me", "keep@example.com"))
            .unwrap();
        let invalid = Contact {
            id: String::new(),
            display_name: "Invalid".to_string(),
            notes: String::new(),
            is_favorite: false,
            emails: vec![],
            created_at: 1,
            updated_at: 1,
        };

        store
            .with_write(|conn| {
                let tx = conn.unchecked_transaction()?;
                assert!(replace_contacts_with_conn(&tx, &[invalid]).is_err());
                let remaining: i64 =
                    tx.query_row("SELECT COUNT(*) FROM contacts", [], |row| row.get(0))?;
                assert_eq!(remaining, 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn contact_duplicate_email_is_case_insensitive_and_atomic() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .save_contact(&contact_input("Alice", "Alice@Example.com"))
            .unwrap();

        let duplicate = store.save_contact(&contact_input("Other Alice", "alice@example.COM"));
        assert!(matches!(duplicate, Err(PebbleError::Validation(_))));

        let contacts = store.list_contacts(None, false, 20, 0).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].id, first.id);
    }

    #[test]
    fn contact_edit_replaces_emails_and_preserves_created_at() {
        let store = Store::open_in_memory().unwrap();
        let created = store
            .save_contact(&contact_input("Alice", "old@example.com"))
            .unwrap();
        let updated = store
            .save_contact(&ContactInput {
                id: Some(created.id.clone()),
                display_name: "Alice Updated".to_string(),
                notes: "New note".to_string(),
                is_favorite: true,
                emails: vec![ContactEmailInput {
                    id: None,
                    address: "new@example.com".to_string(),
                    label: ContactEmailLabel::Work,
                    is_primary: true,
                }],
            })
            .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at >= created.updated_at);
        assert_eq!(updated.emails.len(), 1);
        assert_eq!(updated.emails[0].address, "new@example.com");
    }

    #[test]
    fn contact_list_searches_filters_favorites_and_paginates() {
        let store = Store::open_in_memory().unwrap();
        let alice = store
            .save_contact(&contact_input("Alice", "alice@example.com"))
            .unwrap();
        let mut bob_input = contact_input("Bob", "bob@work.test");
        bob_input.is_favorite = true;
        let bob = store.save_contact(&bob_input).unwrap();
        store
            .save_contact(&contact_input("Charlie", "charlie@example.net"))
            .unwrap();

        let by_name = store.list_contacts(Some("ali"), false, 20, 0).unwrap();
        assert_eq!(
            by_name.iter().map(|c| &c.id).collect::<Vec<_>>(),
            vec![&alice.id]
        );

        let by_email = store
            .list_contacts(Some("work.test"), false, 20, 0)
            .unwrap();
        assert_eq!(
            by_email.iter().map(|c| &c.id).collect::<Vec<_>>(),
            vec![&bob.id]
        );

        let favorites = store.list_contacts(None, true, 20, 0).unwrap();
        assert_eq!(
            favorites.iter().map(|c| &c.id).collect::<Vec<_>>(),
            vec![&bob.id]
        );

        let page = store.list_contacts(None, false, 1, 1).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].display_name, "Bob");
    }

    #[test]
    fn backup_contact_loader_reads_all_pages_from_a_transaction() {
        let store = Store::open_in_memory().unwrap();
        for index in 0..201 {
            store
                .save_contact(&contact_input(
                    &format!("Contact {index:03}"),
                    &format!("contact-{index}@example.com"),
                ))
                .unwrap();
        }

        store
            .with_read(|conn| {
                let tx = conn.unchecked_transaction()?;
                let contacts = list_all_contacts_for_backup_with_conn(&tx)?;
                assert_eq!(contacts.len(), 201);
                tx.commit()?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn contact_favorite_and_delete_update_persisted_contact() {
        let store = Store::open_in_memory().unwrap();
        let saved = store
            .save_contact(&contact_input("Alice", "alice@example.com"))
            .unwrap();

        store.set_contact_favorite(&saved.id, true).unwrap();
        assert!(store.get_contact(&saved.id).unwrap().unwrap().is_favorite);

        store.delete_contact(&saved.id, false).unwrap();
        assert!(store.get_contact(&saved.id).unwrap().is_none());
    }

    #[test]
    fn contact_suggestions_rank_favorite_saved_then_saved_then_recent() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        let mut favorite = contact_input("Zoe Favorite", "zoe@example.com");
        favorite.is_favorite = true;
        store.save_contact(&favorite).unwrap();
        store
            .save_contact(&contact_input("Alice Saved", "alice@example.com"))
            .unwrap();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "recent",
                from_name: "Recent Person",
                from_address: "recent@example.com",
                to: vec![],
                cc: vec![],
                bcc: vec![],
                date: 300,
            },
        );

        let results = store
            .search_contact_suggestions(&account_id, "", 20)
            .unwrap();
        assert_eq!(
            results
                .iter()
                .map(|item| item.address.as_str())
                .collect::<Vec<_>>(),
            vec!["zoe@example.com", "alice@example.com", "recent@example.com"]
        );
        assert_eq!(results[0].source, ContactSuggestionSource::Saved);
        assert_eq!(results[2].source, ContactSuggestionSource::Recent);
    }

    #[test]
    fn contact_suggestions_deduplicate_saved_and_recent_addresses() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        let saved = store
            .save_contact(&contact_input("Saved Alice", "Alice@Example.com"))
            .unwrap();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "alice-history",
                from_name: "Historical Alice",
                from_address: "alice@example.COM",
                to: vec![],
                cc: vec![],
                bcc: vec![],
                date: 500,
            },
        );

        let results = store
            .search_contact_suggestions(&account_id, "alice", 20)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].contact_id.as_deref(), Some(saved.id.as_str()));
        assert_eq!(results[0].source, ContactSuggestionSource::Saved);
        assert_eq!(results[0].last_interaction_at, Some(500));
    }

    #[test]
    fn contact_suggestions_include_cc_and_bcc_but_filter_current_account() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "all-recipients",
                from_name: "Sender",
                from_address: "sender@example.com",
                to: vec![EmailAddress {
                    name: Some("Me".to_string()),
                    address: "ME@example.com".to_string(),
                }],
                cc: vec![EmailAddress {
                    name: Some("Copy".to_string()),
                    address: "copy@example.com".to_string(),
                }],
                bcc: vec![EmailAddress {
                    name: Some("Blind".to_string()),
                    address: "blind@example.com".to_string(),
                }],
                date: 100,
            },
        );

        let results = store
            .search_contact_suggestions(&account_id, "", 20)
            .unwrap();
        let addresses = results
            .iter()
            .map(|item| item.address.to_lowercase())
            .collect::<Vec<_>>();
        assert!(addresses.contains(&"sender@example.com".to_string()));
        assert!(addresses.contains(&"copy@example.com".to_string()));
        assert!(addresses.contains(&"blind@example.com".to_string()));
        assert!(!addresses.contains(&"me@example.com".to_string()));
    }

    #[test]
    fn recent_contact_suggestions_sort_by_latest_interaction() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        for (remote_id, address, date) in [
            ("older", "older@example.com", 100),
            ("newer", "newer@example.com", 200),
        ] {
            insert_suggestion_message(
                &store,
                &account_id,
                &folder_id,
                SuggestionMessage {
                    remote_id,
                    from_name: "Recent",
                    from_address: address,
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    date,
                },
            );
        }

        let results = store
            .search_contact_suggestions(&account_id, "", 20)
            .unwrap();
        assert_eq!(results[0].address, "newer@example.com");
        assert_eq!(results[1].address, "older@example.com");
    }

    #[test]
    fn recent_contact_search_filters_before_applying_history_limit() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "older-match",
                from_name: "Needle Person",
                from_address: "needle@example.com",
                to: vec![],
                cc: vec![],
                bcc: vec![],
                date: 1,
            },
        );
        for index in 0..1_000 {
            let remote_id = format!("unrelated-{index}");
            let address = format!("unrelated-{index}@example.com");
            insert_suggestion_message(
                &store,
                &account_id,
                &folder_id,
                SuggestionMessage {
                    remote_id: &remote_id,
                    from_name: "Unrelated",
                    from_address: &address,
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    date: 100 + index,
                },
            );
        }

        let results = store
            .search_contact_suggestions(&account_id, "needle", 20)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].address, "needle@example.com");
    }

    #[test]
    fn contact_suggestion_suppression_hides_recent_but_not_saved_contact() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "hidden",
                from_name: "Hidden",
                from_address: "hidden@example.com",
                to: vec![],
                cc: vec![],
                bcc: vec![],
                date: 100,
            },
        );
        store
            .suppress_contact_suggestion("HIDDEN@example.com")
            .unwrap();
        assert!(store
            .search_contact_suggestions(&account_id, "hidden", 20)
            .unwrap()
            .is_empty());

        store
            .save_contact(&contact_input("Saved Hidden", "hidden@example.com"))
            .unwrap();
        let results = store
            .search_contact_suggestions(&account_id, "hidden", 20)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, ContactSuggestionSource::Saved);
    }

    #[test]
    fn suppress_contact_suggestion_rejects_invalid_email() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.suppress_contact_suggestion("not-an-address"),
            Err(PebbleError::Validation(_))
        ));
    }

    #[test]
    fn deleting_contact_can_suppress_its_addresses_from_recent_history() {
        let (store, account_id, folder_id) = setup_suggestion_store();
        let saved = store
            .save_contact(&contact_input("Delete Me", "delete@example.com"))
            .unwrap();
        insert_suggestion_message(
            &store,
            &account_id,
            &folder_id,
            SuggestionMessage {
                remote_id: "delete-history",
                from_name: "Delete Me",
                from_address: "delete@example.com",
                to: vec![],
                cc: vec![],
                bcc: vec![],
                date: 100,
            },
        );

        store.delete_contact(&saved.id, true).unwrap();

        assert!(store
            .search_contact_suggestions(&account_id, "delete", 20)
            .unwrap()
            .is_empty());
    }
}
