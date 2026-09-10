//! The `Value` enum: the type of every configuration value.

use std::fmt;
use std::net::IpAddr;

use indexmap::IndexMap;

/// A configuration value.
///
/// The variant set is closed. YAML floats and nulls have no
/// variant here: they are parse errors, since the spec does not represent
/// them (quote such a value to store it as a string).
///
/// The IP variants are semantic model types. A schema-directed codec decides
/// whether a YAML field uses one of them; the model itself never guesses from
/// the text of a string.
#[derive(Clone, Debug)]
pub enum Value {
    /// A string.
    String(String),
    /// A non-negative integer.
    U64(u64),
    /// A negative integer.
    I64(i64),
    /// A boolean.
    Bool(bool),
    /// An IPv4 or IPv6 address.
    IpAddr(IpAddr),
    /// An IP network: `(address, prefix length)`, e.g. `(10.0.1.50, 24)`.
    ///
    /// The address keeps its host bits (`10.0.1.50/24`, not a masked
    /// `10.0.1.0/24`): an IP-with-prefix in this model is an address with a
    /// prefix length, and masking would silently lose the host part. std's
    /// IP-network types are unavailable (unstable) and would mask the
    /// address anyway, so the pair is stored directly.
    IpNetwork((IpAddr, u8)),
    /// An ordered list of values. Element order is significant:
    /// the kernel uses the first address on an interface as the primary
    /// source address.
    List(Vec<Value>),
    /// An ordered map: insertion order is preserved so serialized
    /// output is deterministic.
    Map(IndexMap<String, Value>),
}

impl PartialEq for Value {
    /// Order-sensitive equality: [`Value::Map`] compares keys and values
    /// positionally in insertion order, so two maps carrying the
    /// same pairs in different orders are *not* equal, because they would
    /// serialize to different YAML text.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::String(a), Value::String(b)) => a == b,
            (Value::U64(a), Value::U64(b)) => a == b,
            (Value::I64(a), Value::I64(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::IpAddr(a), Value::IpAddr(b)) => a == b,
            (Value::IpNetwork(a), Value::IpNetwork(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|((k1, v1), (k2, v2))| k1 == k2 && v1 == v2)
            }
            _ => false,
        }
    }
}

impl Eq for Value {}

impl fmt::Display for Value {
    /// Canonical text for logging: scalars bare (bools as `true`/`false`,
    /// networks with their prefix, strings without quotes), lists as
    /// `[a, b]`, maps as `{k: v}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => f.write_str(s),
            Value::U64(n) => write!(f, "{n}"),
            Value::I64(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::IpAddr(a) => write!(f, "{a}"),
            Value::IpNetwork((a, prefix)) => write!(f, "{a}/{prefix}"),
            Value::List(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use indexmap::IndexMap;

    use crate::Value;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn display_string_bare() {
        assert_eq!(Value::String("hello".to_string()).to_string(), "hello");
    }

    #[test]
    fn display_u64() {
        assert_eq!(Value::U64(9000).to_string(), "9000");
    }

    #[test]
    fn display_i64() {
        assert_eq!(Value::I64(-5).to_string(), "-5");
    }

    #[test]
    fn display_bool() {
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Bool(false).to_string(), "false");
    }

    #[test]
    fn display_ip_addr() {
        assert_eq!(Value::IpAddr(ip("10.0.1.5")).to_string(), "10.0.1.5");
        assert_eq!(Value::IpAddr(ip("2001:db8::1")).to_string(), "2001:db8::1");
    }

    #[test]
    fn display_ip_network_keeps_host_bits() {
        let v = Value::IpNetwork((ip("10.0.1.50"), 24));
        // The host part stays visible: 10.0.1.50/24, not a masked 10.0.1.0/24.
        assert_eq!(v.to_string(), "10.0.1.50/24");
    }

    #[test]
    fn display_list() {
        assert_eq!(
            Value::List(vec![Value::U64(1), Value::Bool(true)]).to_string(),
            "[1, true]"
        );
        assert_eq!(Value::List(Vec::new()).to_string(), "[]");
    }

    #[test]
    fn display_map() {
        let mut m = IndexMap::new();
        m.insert("a".to_string(), Value::U64(1));
        m.insert("b".to_string(), Value::Bool(true));
        assert_eq!(Value::Map(m).to_string(), "{a: 1, b: true}");
        assert_eq!(Value::Map(IndexMap::new()).to_string(), "{}");
    }

    #[test]
    fn map_equality_is_order_sensitive() {
        let mut a = IndexMap::new();
        a.insert("x".to_string(), Value::U64(1));
        a.insert("y".to_string(), Value::U64(2));
        let mut b = IndexMap::new();
        b.insert("x".to_string(), Value::U64(1));
        b.insert("y".to_string(), Value::U64(2));
        // Same pairs, same order.
        assert_eq!(Value::Map(a.clone()), Value::Map(b));
        // Same pairs, different insertion order -> not equal (would serialize
        // to different YAML text).
        let mut c = IndexMap::new();
        c.insert("y".to_string(), Value::U64(2));
        c.insert("x".to_string(), Value::U64(1));
        assert_ne!(Value::Map(a.clone()), Value::Map(c));
        // Nested maps: order-sensitive at the nested level too.
        let mut na = IndexMap::new();
        na.insert("x".to_string(), Value::U64(1));
        na.insert("y".to_string(), Value::Map(nested()));
        let mut nc = IndexMap::new();
        nc.insert("y".to_string(), Value::Map(nested()));
        nc.insert("x".to_string(), Value::U64(1));
        assert_ne!(Value::Map(na), Value::Map(nc));
        // Different length.
        let mut d = IndexMap::new();
        d.insert("x".to_string(), Value::U64(1));
        assert_ne!(Value::Map(a), Value::Map(d));
    }

    fn nested() -> IndexMap<String, Value> {
        let mut n = IndexMap::new();
        n.insert("z".to_string(), Value::Bool(false));
        n
    }

    #[test]
    fn cross_variant_inequality() {
        // Pins the `_ => false` fallthrough in Value's PartialEq.
        assert_ne!(Value::U64(1), Value::I64(1));
        assert_ne!(Value::String("1".to_string()), Value::U64(1));
        assert_ne!(
            Value::IpAddr(ip("10.0.1.5")),
            Value::IpNetwork((ip("10.0.1.5"), 24))
        );
        assert_ne!(Value::Bool(true), Value::U64(1));
        assert_ne!(Value::List(vec![Value::U64(1)]), Value::U64(1));
        assert_ne!(Value::Map(IndexMap::new()), Value::List(Vec::new()));
    }
}
