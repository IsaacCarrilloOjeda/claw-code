//! Contact registry — maps phone numbers to names and roles.
//!
//! Primary source: `GHOST_CONTACTS` env var.
//! Format: `+1XXXXXXXXXX:Name:role,+1XXXXXXXXXX:Name:role`
//! Roles: `owner` (Isaac), `allowed` (whitelisted contact)
//!
//! Fallback: `GHOST_ALLOWED_NUMBERS` (all get role `allowed`, name `Unknown`)
//! with `GHOST_OWNER_NUMBER` to identify the owner.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Contact {
    pub number: String, // normalized E.164
    pub name: String,
    pub role: ContactRole,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContactRole {
    Owner,   // Isaac — peer-to-peer mode, no AI disclosure
    Allowed, // Whitelisted contact — representative mode, AI disclosure
}

static CONTACTS: OnceLock<HashMap<String, Contact>> = OnceLock::new();

/// Initialize or return the contact registry. Parsed once, cached forever.
pub fn registry() -> &'static HashMap<String, Contact> {
    CONTACTS.get_or_init(|| {
        if let Ok(raw) = std::env::var("GHOST_CONTACTS") {
            parse_contacts(&raw)
        } else {
            parse_fallback()
        }
    })
}

/// Look up a sender by phone number. Returns None if not in registry.
pub fn lookup(phone: &str) -> Option<&'static Contact> {
    let normalized = crate::daemon::normalize_phone(phone);
    registry().get(&normalized)
}

/// Returns true if the given number belongs to the owner (Isaac).
pub fn is_owner(phone: &str) -> bool {
    lookup(phone).is_some_and(|c| c.role == ContactRole::Owner)
}

/// Build a system prompt fragment describing who is texting.
/// This gets injected into the chat dispatcher's system prompt.
pub fn sender_context(phone: &str) -> String {
    match lookup(phone) {
        Some(contact) if contact.role == ContactRole::Owner => {
            format!(
                "## Who is texting you right now\n\
                 This message is from {} -- your owner. Talk to him peer-to-peer.\n\
                 Do NOT introduce yourself or identify as AI -- he knows who you are.",
                contact.name
            )
        }
        Some(contact) => {
            format!(
                "## Who is texting you right now\n\
                 This message is from {} (phone: {}). They are NOT Isaac.\n\
                 You MUST identify yourself as GHOST, Isaac's AI assistant, in your first reply.\n\
                 Be friendly and helpful, but never pretend to be Isaac.\n\
                 If they ask to speak to Isaac directly, say you'll make sure he sees their message.",
                contact.name, contact.number
            )
        }
        None => {
            format!(
                "## Who is texting you right now\n\
                 This message is from an unknown number: {phone}. They are NOT Isaac.\n\
                 You MUST identify yourself as GHOST, Isaac's AI assistant.\n\
                 Be helpful but cautious with unknown contacts.\n\
                 Do not share any personal information about Isaac."
            )
        }
    }
}

fn parse_contacts(raw: &str) -> HashMap<String, Contact> {
    let mut map = HashMap::new();
    for entry in raw.split(',') {
        let parts: Vec<&str> = entry.trim().splitn(3, ':').collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }
        let number = crate::daemon::normalize_phone(parts[0]);
        let name = parts.get(1).copied().unwrap_or("Unknown").to_string();
        let role = match parts.get(2).copied().unwrap_or("allowed") {
            "owner" => ContactRole::Owner,
            _ => ContactRole::Allowed,
        };
        map.insert(number.clone(), Contact { number, name, role });
    }
    map
}

fn parse_fallback() -> HashMap<String, Contact> {
    let mut map = HashMap::new();
    let owner_num = std::env::var("GHOST_OWNER_NUMBER")
        .ok()
        .map(|n| crate::daemon::normalize_phone(&n));

    if let Ok(allowed) = std::env::var("GHOST_ALLOWED_NUMBERS") {
        for entry in allowed.split(',') {
            let number = crate::daemon::normalize_phone(entry.trim());
            if number.is_empty() {
                continue;
            }
            let is_owner = owner_num.as_deref() == Some(number.as_str());
            let role = if is_owner {
                ContactRole::Owner
            } else {
                ContactRole::Allowed
            };
            let name = if is_owner {
                "Isaac".to_string()
            } else {
                "Unknown".to_string()
            };
            map.insert(number.clone(), Contact { number, name, role });
        }
    }
    map
}
