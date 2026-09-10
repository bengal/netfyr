//! The `State` type: one source's contribution to one device.

use std::time::SystemTime;

use indexmap::IndexMap;

use crate::match_spec::Match;
use crate::source::Source;
use crate::value::Value;

/// Runtime-only bookkeeping for a [`State`]. Never part of the YAML format:
/// a fresh instance is created on every deserialization.
#[derive(Clone, Debug)]
pub struct StateMetadata {
    /// A unique in-memory handle, generated as a UUIDv7.
    pub id: uuid::Uuid,
    /// When this in-memory state object was built.
    pub created_at: SystemTime,
}

impl Default for StateMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMetadata {
    /// Create fresh metadata with a new UUIDv7 and the current time.
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::now_v7(),
            created_at: SystemTime::now(),
        }
    }
}

/// One source's contribution to one device.
///
/// The same type is used for *desired* state (a policy or dynamic
/// provider's contribution, typically a partial match and empty
/// `device_type`) and for *actual* state (queried from the kernel, a fully
/// populated match and `device_type`).
///
/// Configuration is a plain ordered `Vec<State>`: contributions to the same
/// device coexist and are never merged, deduplicated, or rejected here.
/// Merging requires resolving each contribution's match against the live
/// system, which is the reconciliation layer's job.
///
/// No `PartialEq` is derived, because `metadata` contains a fresh UUID and
/// timestamp. Compare non-runtime model content with [`State::content_eq`].
#[derive(Clone, Debug)]
pub struct State {
    /// The technology type of the device (e.g. `"ethernet"`). Empty for
    /// desired-state contributions that neither set `type:` nor carry a
    /// technology sub-object. The model does not distinguish an explicit
    /// `type:` from one inferred from a technology sub-object.
    pub device_type: String,
    /// Which device this state targets.
    pub match_spec: Match,
    /// The contributed configuration, in insertion order.
    pub fields: IndexMap<String, Value>,
    /// Which source produced this contribution.
    pub source: Source,
    /// Conflict-resolution weight: higher wins. Defaults to
    /// [`Source::default_priority`]; the producing policy or provider may
    /// override it.
    pub priority: i32,
    /// Runtime-only bookkeeping.
    pub metadata: StateMetadata,
}

impl State {
    /// Create a state with the given source, that source's default
    /// priority, fresh metadata, and no device type, match, or fields.
    pub fn new(source: Source) -> Self {
        let priority = source.default_priority();
        Self {
            device_type: String::new(),
            match_spec: Match::default(),
            fields: IndexMap::new(),
            source,
            priority,
            metadata: StateMetadata::new(),
        }
    }

    /// Whether the two states carry the same non-runtime model content:
    /// `device_type`, `match_spec`, and `fields` (including field order).
    /// `source`, `priority`, and `metadata` are deliberately excluded.
    pub fn content_eq(&self, other: &State) -> bool {
        self.device_type == other.device_type
            && self.match_spec == other.match_spec
            // `IndexMap`'s `PartialEq` ignores insertion order, but order is
            // part of the content: compare keys and values positionally so
            // reordered fields are not equal.
            && self.fields.len() == other.fields.len()
            && self
                .fields
                .iter()
                .zip(other.fields.iter())
                .all(|((k1, v1), (k2, v2))| k1 == k2 && v1 == v2)
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use indexmap::IndexMap;

    use crate::{Match, Source, State, StateMetadata, Value};

    fn state_with(device_type: &str, fields: IndexMap<String, Value>) -> State {
        State {
            device_type: device_type.to_string(),
            match_spec: Match::default(),
            fields,
            source: Source::Static {
                policy: String::new(),
            },
            priority: 100,
            metadata: StateMetadata::new(),
        }
    }

    fn two_fields() -> IndexMap<String, Value> {
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        f.insert("enabled".to_string(), Value::Bool(true));
        f
    }

    #[test]
    fn new_derives_priority_and_empty_content() {
        for (src, expected) in [
            (
                Source::Static {
                    policy: String::new(),
                },
                100,
            ),
            (
                Source::Vpn {
                    policy: String::new(),
                },
                75,
            ),
            (
                Source::Dhcp {
                    policy: String::new(),
                },
                50,
            ),
            (
                Source::Ra {
                    policy: String::new(),
                },
                25,
            ),
            (Source::Kernel, 0),
        ] {
            let s = State::new(src.clone());
            assert_eq!(s.priority, expected, "source {src:?}");
            assert!(s.device_type.is_empty());
            assert!(s.match_spec.is_empty());
            assert!(s.fields.is_empty());
        }
    }

    #[test]
    fn new_generates_fresh_metadata() {
        let before = SystemTime::now();
        let s = State::new(Source::Kernel);
        let after = SystemTime::now();
        // UUIDv7, with the standard RFC 4122 variant.
        assert_eq!(s.metadata.id.get_version_num(), 7);
        assert_eq!(s.metadata.id.get_variant(), uuid::Variant::RFC4122);
        // Bounded time, never an exact comparison.
        assert!(s.metadata.created_at >= before);
        assert!(s.metadata.created_at <= after);
        // Two freshly generated metadata carry different ids.
        let a = StateMetadata::new();
        let b = StateMetadata::new();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn content_eq_true_for_same_content_different_runtime_fields() {
        let a = state_with("ethernet", two_fields());
        let mut b = state_with("ethernet", two_fields());
        b.source = Source::Kernel;
        b.priority = 0;
        b.metadata = StateMetadata::new();
        // Same device_type/match_spec/fields -> content equal despite runtime
        // fields (and metadata) differing.
        assert!(a.content_eq(&b));
    }

    #[test]
    fn content_eq_false_on_device_type() {
        let a = state_with("ethernet", two_fields());
        let b = state_with("wifi", two_fields());
        assert!(!a.content_eq(&b));
    }

    #[test]
    fn content_eq_false_on_match_spec() {
        let mut a = state_with("ethernet", two_fields());
        let mut b = state_with("ethernet", two_fields());
        a.match_spec.name = Some("eth0".to_string());
        b.match_spec.name = Some("eth1".to_string());
        assert!(!a.content_eq(&b));
    }

    #[test]
    fn content_eq_false_on_field_value() {
        let a = state_with("ethernet", two_fields());
        let mut b = state_with("ethernet", two_fields());
        // Re-inserting an existing key updates the value in place, so only the
        // value differs, not the order.
        b.fields.insert("mtu".to_string(), Value::U64(1500));
        assert!(!a.content_eq(&b));
    }

    #[test]
    fn content_eq_false_on_field_key_set() {
        let mut a = state_with("ethernet", IndexMap::new());
        a.fields.insert("mtu".to_string(), Value::U64(9000));
        let mut b = state_with("ethernet", IndexMap::new());
        b.fields.insert("mtu".to_string(), Value::U64(9000));
        b.fields.insert("extra".to_string(), Value::Bool(true));
        assert!(!a.content_eq(&b));
    }

    #[test]
    fn content_eq_false_on_field_order() {
        // Same pairs in different insertion order are NOT equal content
        // (IndexMap::PartialEq would say they are; content_eq
        // must not).
        let mut a = IndexMap::new();
        a.insert("first".to_string(), Value::U64(1));
        a.insert("second".to_string(), Value::U64(2));
        let mut b = IndexMap::new();
        b.insert("second".to_string(), Value::U64(2));
        b.insert("first".to_string(), Value::U64(1));
        let s1 = state_with("ethernet", a);
        let s2 = state_with("ethernet", b);
        assert!(!s1.content_eq(&s2));
    }
}
