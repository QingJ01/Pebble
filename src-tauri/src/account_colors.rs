use pebble_core::Account;
use std::collections::HashSet;

// Keep in sync with src/lib/accountColors.ts and the copy in
// crates/pebble-store/src/migrations/mod.rs. No orange, amber or red: those
// read as brand, star/warning and danger.
const ACCOUNT_COLOR_PRESETS: [&str; 12] = [
    "#0ea5e9", "#10b981", "#8b5cf6", "#ec4899", "#65a30d", "#6366f1", "#14b8a6", "#c026d3",
    "#3b82f6", "#64748b", "#06b6d4", "#1e40af",
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
            account_label: None,
            provider_display_name: None,
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
            account("two", Some("#10b981")),
        ];

        assert_eq!(
            super::default_account_color(&accounts, "three@example.com"),
            "#8b5cf6"
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

    #[test]
    fn presets_avoid_brand_warning_and_danger_hues() {
        for color in super::ACCOUNT_COLOR_PRESETS {
            let hex = u32::from_str_radix(&color[1..], 16).expect("valid hex");
            let [r, g, b] =
                [(hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff].map(|c| c as f64 / 255.0);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            if delta < 0.15 {
                continue; // near-neutral (slate)
            }
            let hue = if max == r {
                60.0 * ((g - b) / delta).rem_euclid(6.0)
            } else if max == g {
                60.0 * ((b - r) / delta + 2.0)
            } else {
                60.0 * ((r - g) / delta + 4.0)
            };
            // Red (danger), orange (brand) and amber (star/warning) live in 345°..60°.
            assert!(
                (60.0..345.0).contains(&hue),
                "{color} (hue {hue:.0}) is in the red/orange/amber family"
            );
        }
    }
}
