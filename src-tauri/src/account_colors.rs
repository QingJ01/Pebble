use pebble_core::Account;
use std::collections::HashSet;

const ACCOUNT_COLOR_PRESETS: [&str; 12] = [
    "#0ea5e9", "#22c55e", "#f59e0b", "#8b5cf6", "#f43f5e", "#14b8a6", "#6366f1", "#f97316",
    "#06b6d4", "#ec4899", "#84cc16", "#3b82f6",
];

fn is_valid_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.as_bytes()[0] == b'#'
        && color.as_bytes()[1..].iter().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn default_account_color(existing_accounts: &[Account], seed: &str) -> String {
    let used_colors: HashSet<String> = existing_accounts
        .iter()
        .filter_map(|account| account.color.as_deref())
        .filter(|color| is_valid_hex_color(color))
        .map(str::to_ascii_lowercase)
        .collect();

    if let Some(color) = ACCOUNT_COLOR_PRESETS
        .iter()
        .find(|color| !used_colors.contains(**color))
    {
        return (*color).to_string();
    }

    let mut hash = 0u32;
    for byte in seed.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }

    ACCOUNT_COLOR_PRESETS[(hash as usize) % ACCOUNT_COLOR_PRESETS.len()].to_string()
}

#[cfg(test)]
mod tests {
    use pebble_core::{now_timestamp, Account, ProviderType};

    fn account(id: &str, color: Option<&str>) -> Account {
        let now = now_timestamp();
        Account {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            display_name: id.to_string(),
            color: color.map(ToOwned::to_owned),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn default_account_color_uses_the_first_unused_preset() {
        let accounts = vec![
            account("one", Some("#0ea5e9")),
            account("two", Some("#22c55e")),
        ];

        assert_eq!(
            super::default_account_color(&accounts, "three@example.com"),
            "#f59e0b"
        );
    }

    #[test]
    fn default_account_color_ignores_invalid_saved_colors() {
        let accounts = vec![account("one", Some("not-a-color"))];

        assert_eq!(
            super::default_account_color(&accounts, "two@example.com"),
            "#0ea5e9"
        );
    }
}
