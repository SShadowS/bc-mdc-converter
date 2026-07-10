// Shared AL identifier-quoting + al-preview:// URL synthesis (algorithm spec §7.5
// `QuoteIdentifierIfNeeded`, §7.7 `CreateUri`). Promoted out of `v8_json.rs` in FT0 (Plan 2)
// so the Firefox emitter can reuse the exact same logic — bodies below are unchanged
// verbatim from their original `v8_json.rs` home; `v8_json.rs` now re-exports them so the
// V8 emitter's call sites (`v8_json::build_url` / `v8_json::quote_identifier_if_needed`)
// keep working unmodified. The V8 byte-match depends on this being a pure move.

use crate::mdc::types::ApplicationObjectId;

/// algorithm spec §7.7 `CreateUri` / the `al-preview://` fallback (no workspace compilation in headless
/// conversion, so this branch is always taken). `app_id` is the declaring app's GUID, already
/// stored lowercase 32-hex ("N" format) in the fixture — emitted as-is, not re-normalized.
/// `project_name` is the local-project fallback (`EmitOptions::project_name`), used only when
/// `app_id` is absent/empty.
pub(crate) fn build_url(id: ApplicationObjectId, app_id: Option<&str>, project_name: Option<&str>) -> String {
    let kind = id.object_type.symbol_kind(); // NOTE: `Codeunit`, NOT the scriptId's `CodeUnit`.
    let name = id.to_script_id();

    let prefix = match app_id {
        Some(a) if !a.is_empty() => format!("{}/", a),
        _ => match project_name {
            Some(p) if !p.is_empty() => format!("{}/", escape(p)),
            _ => String::new(),
        },
    };

    format!(
        "al-preview://allang/{prefix}{kind}/{number}/{escaped}.dal",
        prefix = prefix,
        kind = kind,
        number = id.number,
        escaped = escape(&name),
    )
}

/// `PreviewFileUrl.Escape` (algorithm spec §7.7): replace literal `/` with U+2044 FRACTION SLASH, then
/// percent-encode everything outside the unreserved set. A no-op for `Type_Number` names (the
/// only shape this converter ever synthesizes) since those are pure ASCII alnum + `_`.
fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let c = if ch == '/' { '\u{2044}' } else { ch };
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

/// algorithm spec §7.5 `QuoteIdentifierIfNeeded`: unquoted iff `name` matches
/// `^[A-Za-z_][A-Za-z0-9_]*$` AND is not an AL reserved word; else wrap in `"`, doubling any
/// internal `"`.
pub(crate) fn quote_identifier_if_needed(name: &str) -> String {
    let is_simple_identifier = {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            _ => false,
        }
    };

    if is_simple_identifier && !is_al_keyword(name) {
        name.to_string()
    } else {
        let mut out = String::with_capacity(name.len() + 2);
        out.push('"');
        for c in name.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
        out
    }
}

/// AL object descriptor: `<TypeDisplay> <id> <QuotedName>`, e.g. `Codeunit 80 "Sales-Post"`.
/// `quote_identifier_if_needed` ALREADY supplies the surrounding quotes for non-simple names
/// (and none for simple identifiers) — do NOT add more quotes around it.
pub(crate) fn al_object_descriptor(type_display: &str, id: i32, object_name: &str) -> String {
    format!("{} {} {}", type_display, id, quote_identifier_if_needed(object_name))
}

fn is_al_keyword(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    AL_KEYWORDS.contains(&upper.as_str())
}

/// AL reserved words (case-insensitive, per the AL language reference). Covers the general
/// statement/expression keywords, object-type words, and built-in type names that a bare
/// object/procedure name could otherwise collide with. A name containing a space/dot/hyphen/etc.
/// is always quoted regardless of this set (that's the common case exercised by every fixture);
/// this list matters only for the edge case of a *single-word* name that happens to also be a
/// keyword.
// best-effort: compiled from the AL language reference; not guaranteed exhaustive.
static AL_KEYWORDS: &[&str] = &[
    "AND", "ARRAY", "ASSERTERROR", "BEGIN", "BIGINTEGER", "BIGTEXT", "BLOB", "BOOLEAN", "BREAK",
    "BYTE", "CASE", "CHAR", "CODE", "CODEUNIT", "CONTROLADDIN", "DATABASE", "DATE", "DATEFORMULA",
    "DATETIME", "DECIMAL", "DICTIONARY", "DIVIDE", "DO", "DOTNET", "DOWNTO", "DURATION", "ELSE",
    "END", "ENTITLEMENT", "ENUM", "ENUMEXTENSION", "EVENT", "EXIT", "EXTENDS", "FIELD",
    "FIELDGROUP", "FIELDREF", "FIELDS", "FOR", "FOREACH", "FUNCTION", "GUID", "IF", "IMPLEMENTS",
    "IN", "INDATASET", "INSTREAM", "INTEGER", "INTERFACE", "INTERNAL", "JSONARRAY", "JSONOBJECT",
    "JSONTOKEN", "JSONVALUE", "KEY", "KEYREF", "KEYS", "LABEL", "LIST", "LOCAL", "MOD",
    "NAMESPACE", "NOT", "OF", "OPTION", "OR", "OUTSTREAM", "PAGE", "PAGEEXTENSION", "PERMISSIONSET",
    "PERMISSIONSETEXTENSION", "PRAGMA", "PROCEDURE", "PROFILE", "PROGRAM", "PROTECTED", "QUERY",
    "RECORD", "RECORDID", "RECORDREF", "REPEAT", "REPORT", "REPORTEXTENSION", "REQUESTPAGE",
    "RUNONCLIENT", "SECURITYFILTERING", "SUPPRESSDISPOSE", "TABLE", "TABLEEXTENSION", "TEMPORARY",
    "TESTPAGE", "TESTREQUESTPAGE", "TEXT", "TEXTBUILDER", "TEXTCONST", "THEN", "TO", "TRIGGER",
    "TRUE", "FALSE", "UNTIL", "USING", "VAR", "VARIANT", "VERSION", "WHILE", "WITH", "WITHEVENTS",
    "XMLPORT", "XOR",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdc::types::ObjectType;

    #[test]
    fn quote_identifier_leaves_simple_names_bare() {
        assert_eq!(quote_identifier_if_needed("Customer"), "Customer");
        assert_eq!(quote_identifier_if_needed("Item"), "Item");
        assert_eq!(quote_identifier_if_needed("Language"), "Language");
    }

    #[test]
    fn quote_identifier_wraps_names_with_spaces_or_dots() {
        assert_eq!(
            quote_identifier_if_needed("User Settings Impl."),
            "\"User Settings Impl.\""
        );
        assert_eq!(
            quote_identifier_if_needed("System Action Triggers"),
            "\"System Action Triggers\""
        );
    }

    #[test]
    fn quote_identifier_wraps_bare_al_keywords() {
        // The keyword branch (a name that IS a simple identifier but collides with a reserved
        // word) — quoted despite matching the regex. Case-insensitive.
        assert_eq!(quote_identifier_if_needed("Table"), "\"Table\"");
        assert_eq!(quote_identifier_if_needed("Codeunit"), "\"Codeunit\"");
        assert_eq!(quote_identifier_if_needed("begin"), "\"begin\"");
        // A normal identifier that is not a keyword stays unquoted (contrast).
        assert_eq!(quote_identifier_if_needed("Customer"), "Customer");
    }

    #[test]
    fn build_url_uses_lowercase_host_and_symbol_kind_casing() {
        let id = ApplicationObjectId { number: 2000000006, object_type: ObjectType::CodeUnit };
        let url = build_url(id, Some("8874ed3a064342479ced7a7002f7135d"), None);
        assert_eq!(
            url,
            "al-preview://allang/8874ed3a064342479ced7a7002f7135d/Codeunit/2000000006/CodeUnit_2000000006.dal"
        );
    }
}
