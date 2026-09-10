//! The two YAML formats (query output and policy) and their conversion to
//! the model.
//!
//! All YAML handling in the crate lives in this module and works in two
//! phases: text ↔ `serde_yaml::Value` ↔ the netfyr model. The model types
//! carry no serde dependency, and `serde_yaml` does not appear anywhere
//! else in the crate, so swapping YAML libraries later is a one-module
//! change.
//!
//! Parsing traverses the embedded schema with the YAML tree. A scalar is
//! interpreted only by the schema node governing its field path; the codec
//! never infers an IP value solely from string contents.
//!
//! A state's `kind`, `source`, `priority`, and metadata are not part of the
//! YAML format: a state deserialized from YAML is a bare
//! state with [`Source::Static`], the default static priority, and fresh
//! metadata.

use indexmap::IndexMap;

use crate::error::Error;
use crate::match_spec::Match;
use crate::schema::{EMBEDDED_REGISTRY, SchemaCursor, SchemaRegistry};
use crate::source::Source;
use crate::state::{State, StateMetadata};
use crate::value::Value;

/// Deserialize a YAML document into an ordered list of states.
///
/// A document whose top level is a mapping produces exactly one state; a
/// document whose top level is a sequence produces one state per element,
/// in list order. A `---` separator starting a second document is
/// an error; a leading `---` on the single document is accepted. Empty or
/// whitespace-only input is an error; an explicit `[]` yields zero states.
pub fn from_yaml(input: &str) -> Result<Vec<State>, Error> {
    EMBEDDED_REGISTRY.from_yaml(input)
}

/// Schema-aware implementation behind [`from_yaml`] and
/// [`SchemaRegistry::from_yaml`].
pub(crate) fn from_yaml_with_registry(
    input: &str,
    registry: &SchemaRegistry,
) -> Result<Vec<State>, Error> {
    if input.trim().is_empty() {
        return Err(Error::EmptyInput);
    }
    let doc = match serde_yaml::from_str::<serde_yaml::Value>(input) {
        Ok(doc) => doc,
        Err(err) => {
            // serde_yaml has no public error-kind API; its multi-document
            // diagnostic is the only signal available.
            let message = err.to_string();
            if message.contains("more than one document") {
                return Err(Error::MultiDocument);
            }
            return Err(Error::Yaml(message));
        }
    };
    match doc {
        serde_yaml::Value::Mapping(m) => Ok(vec![mapping_to_state(&m, registry)?]),
        serde_yaml::Value::Sequence(entries) => entries
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                serde_yaml::Value::Mapping(m) => mapping_to_state(m, registry),
                other => Err(Error::InvalidTopLevel(format!(
                    "list element {i} is {}, not a mapping",
                    shape(other)
                ))),
            })
            .collect(),
        other => Err(Error::InvalidTopLevel(shape(&other).to_string())),
    }
}

/// Serialize states in the query output format (what `netfyr query`
/// prints): base and grouped fields at the top level, no `match:` section.
pub fn to_yaml_query(states: &[State]) -> Result<String, Error> {
    EMBEDDED_REGISTRY.to_yaml_query(states)
}

/// Schema-aware implementation behind [`to_yaml_query`] and
/// [`SchemaRegistry::to_yaml_query`].
pub(crate) fn to_yaml_query_with_registry(
    states: &[State],
    registry: &SchemaRegistry,
) -> Result<String, Error> {
    render(states, false, registry)
}

/// Serialize states in the policy format (what the user writes for
/// `netfyr apply`): a `match:` section identifying the target, followed by
/// the same fields as the query format.
pub fn to_yaml_policy(states: &[State]) -> Result<String, Error> {
    EMBEDDED_REGISTRY.to_yaml_policy(states)
}

/// Schema-aware implementation behind [`to_yaml_policy`] and
/// [`SchemaRegistry::to_yaml_policy`].
pub(crate) fn to_yaml_policy_with_registry(
    states: &[State],
    registry: &SchemaRegistry,
) -> Result<String, Error> {
    render(states, true, registry)
}

fn render(
    states: &[State],
    include_match: bool,
    registry: &SchemaRegistry,
) -> Result<String, Error> {
    // One state serializes as a single top-level mapping; two or more as a
    // top-level sequence, one entry per state, in list order.
    let doc = if states.len() == 1 {
        state_to_mapping(&states[0], 0, include_match, registry)?
    } else {
        serde_yaml::Value::Sequence(
            states
                .iter()
                .enumerate()
                .map(|(state_index, state)| {
                    state_to_mapping(state, state_index, include_match, registry)
                })
                .collect::<Result<_, _>>()?,
        )
    };
    serde_yaml::to_string(&doc).map_err(|err| Error::Yaml(err.to_string()))
}

fn state_to_mapping(
    state: &State,
    state_index: usize,
    include_match: bool,
    registry: &SchemaRegistry,
) -> Result<serde_yaml::Value, Error> {
    for key in state.fields.keys() {
        if matches!(key.as_str(), "match" | "type" | "kind") {
            return Err(Error::ReservedFieldKey {
                key: key.clone(),
                state_index,
            });
        }
    }

    let mut m = serde_yaml::Mapping::new();
    if include_match {
        let mut match_map = serde_yaml::Mapping::new();
        // Fixed sub-key order; `None` fields are omitted. The empty mapping
        // is still emitted so policy YAML remains distinguishable from query
        // YAML; policy-mode decoding rejects it as an incomplete selector.
        for (key, value) in [
            ("name", &state.match_spec.name),
            ("type", &state.match_spec.r#type),
            ("driver", &state.match_spec.driver),
            ("pci_path", &state.match_spec.pci_path),
            ("mac", &state.match_spec.mac),
        ] {
            if let Some(v) = value {
                match_map.insert(
                    serde_yaml::Value::String(key.to_string()),
                    serde_yaml::Value::String(v.clone()),
                );
            }
        }
        m.insert(
            serde_yaml::Value::String("match".to_string()),
            serde_yaml::Value::Mapping(match_map),
        );
    }
    // The policy format omits a `type:` that a technology sub-object already
    // implies: a policy constrains the type through `match:` instead of
    // setting it, and restating it would stop the serializer's own
    // output from being a fixed point (re-parsing re-derives the type from
    // the sub-object, so the next pass would emit a redundant `type:` the
    // first pass did not). The query format always states the type because
    // it is a full report of the device.
    let type_is_implied =
        registry.implied_device_type(&state.fields) == Some(state.device_type.as_str());
    let omit_type = state.device_type.is_empty() || (include_match && type_is_implied);
    if !omit_type {
        m.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String(state.device_type.clone()),
        );
    }
    for (key, value) in &state.fields {
        m.insert(
            serde_yaml::Value::String(key.clone()),
            model_to_value(value, registry.top_level_schema(key)),
        );
    }
    Ok(serde_yaml::Value::Mapping(m))
}

/// Convert one device mapping to a state: route the reserved top-level
/// keys, convert the rest to `fields`, and apply the technology
/// sub-object → device type implication.
fn mapping_to_state(m: &serde_yaml::Mapping, registry: &SchemaRegistry) -> Result<State, Error> {
    let mut match_spec = Match::default();
    let mut device_type = String::new();
    let mut fields: IndexMap<String, Value> = IndexMap::new();

    for (key, value) in m.iter() {
        let key = key_to_string(key)?;
        match key.as_str() {
            "match" => match_spec = parse_match(value)?,
            "kind" => parse_kind(value)?,
            "type" => {
                device_type = value
                    .as_str()
                    .ok_or_else(|| Error::UnsupportedScalar {
                        kind: shape(value).to_string(),
                    })?
                    .to_string();
            }
            // Unknown fields stay representable for later diagnostics, but
            // registered paths are decoded with their schema immediately.
            _ => {
                fields.insert(
                    key.clone(),
                    value_to_model(value, registry.top_level_schema(&key))?,
                );
            }
        }
    }

    // A technology sub-object implies the device type when it is not
    // explicitly set; an explicit `type:` wins.
    if device_type.is_empty() {
        if let Some(implied) = registry.implied_device_type(&fields) {
            device_type = implied.to_string();
        }
    }

    let source = Source::Static {
        policy: String::new(),
    };
    let priority = source.default_priority();
    Ok(State {
        device_type,
        match_spec,
        fields,
        source,
        priority,
        metadata: StateMetadata::new(),
    })
}

/// Render a mapping key to its string form. String keys are used as-is;
/// scalar keys (e.g. `9000:`) take their canonical text, since field keys
/// are open and validated by the schema spec; null, sequence, mapping, and
/// tagged keys have no usable string form and are an error.
fn key_to_string(key: &serde_yaml::Value) -> Result<String, Error> {
    match key {
        serde_yaml::Value::String(s) => Ok(s.clone()),
        serde_yaml::Value::Number(n) => Ok(n.to_string()),
        serde_yaml::Value::Bool(b) => Ok(b.to_string()),
        other => Err(Error::InvalidFieldKey(shape(other).to_string())),
    }
}

/// Validate the `kind` discriminator and consume it: absent or `state` is a
/// bare state, `policy` is the policy format (parsed elsewhere), anything
/// else is an error.
fn parse_kind(value: &serde_yaml::Value) -> Result<(), Error> {
    match value {
        serde_yaml::Value::String(kind) if kind == "state" => Ok(()),
        serde_yaml::Value::String(kind) if kind == "policy" => {
            Err(Error::PolicyWrapperNotSupported)
        }
        serde_yaml::Value::String(kind) => Err(Error::UnknownKind(kind.clone())),
        other => Err(Error::UnknownKind(shape(other).to_string())),
    }
}

/// Parse a `match:` section. The sub-keys are a closed set (this spec fully
/// defines `Match`), and values must be string scalars.
fn parse_match(value: &serde_yaml::Value) -> Result<Match, Error> {
    let m = value.as_mapping().ok_or(Error::MatchNotMapping)?;
    let mut match_spec = Match::default();
    for (key, value) in m.iter() {
        let key = key_to_string(key)?;
        let s = value
            .as_str()
            .ok_or_else(|| Error::MatchValueNotString { key: key.clone() })?;
        match key.as_str() {
            "name" => match_spec.name = Some(s.to_string()),
            "type" => match_spec.r#type = Some(s.to_string()),
            "driver" => match_spec.driver = Some(s.to_string()),
            "pci_path" => match_spec.pci_path = Some(s.to_string()),
            "mac" => match_spec.mac = Some(s.to_string()),
            other => return Err(Error::UnknownMatchKey(other.to_string())),
        }
    }
    Ok(match_spec)
}

/// Convert a YAML value using the schema governing this path. Strings become
/// semantic values only where a declared schema format requests it; unknown
/// paths deliberately use no schema and never infer types from their text.
fn value_to_model(v: &serde_yaml::Value, schema: Option<SchemaCursor<'_>>) -> Result<Value, Error> {
    match v {
        serde_yaml::Value::Null => Err(Error::UnsupportedScalar {
            kind: "null".to_string(),
        }),
        serde_yaml::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_yaml::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Ok(Value::U64(u))
            } else if let Some(i) = n.as_i64() {
                Ok(Value::I64(i))
            } else {
                Err(Error::UnsupportedScalar {
                    kind: "float".to_string(),
                })
            }
        }
        serde_yaml::Value::String(s) => {
            Ok(schema.map_or_else(|| Value::String(s.clone()), |node| node.decode_string(s)))
        }
        serde_yaml::Value::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(value_to_model(item, schema.and_then(SchemaCursor::items))?);
            }
            Ok(Value::List(out))
        }
        serde_yaml::Value::Mapping(m) => {
            let mut out = IndexMap::with_capacity(m.len());
            for (k, item) in m.iter() {
                let key = key_to_string(k)?;
                out.insert(
                    key.clone(),
                    value_to_model(item, schema.and_then(|node| node.property(&key)))?,
                );
            }
            Ok(Value::Map(out))
        }
        // A YAML tag (e.g. `!!str`) is not part of the format; treating it
        // as a plain string would hide what the tag declared.
        serde_yaml::Value::Tagged(_) => Err(Error::Yaml(
            "tagged YAML values are not supported".to_string(),
        )),
    }
}

/// Convert a model [`Value`] to its YAML form. String spelling does not carry
/// semantic IP information; schema-directed parsing recovers it by path.
fn model_to_value(v: &Value, schema: Option<SchemaCursor<'_>>) -> serde_yaml::Value {
    match v {
        Value::String(s) => serde_yaml::Value::String(s.clone()),
        Value::U64(n) => serde_yaml::Value::Number(serde_yaml::Number::from(*n)),
        Value::I64(n) => serde_yaml::Value::Number(serde_yaml::Number::from(*n)),
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        // IP values serialize to canonical text. Only a matching formatted
        // schema path reconstitutes the specialized model variant.
        Value::IpAddr(a) => serde_yaml::Value::String(a.to_string()),
        Value::IpNetwork((a, prefix)) => serde_yaml::Value::String(format!("{a}/{prefix}")),
        Value::List(items) => serde_yaml::Value::Sequence(
            items
                .iter()
                .map(|item| model_to_value(item, schema.and_then(SchemaCursor::items)))
                .collect(),
        ),
        Value::Map(entries) => {
            let mut m = serde_yaml::Mapping::with_capacity(entries.len());
            for (k, item) in entries {
                m.insert(
                    serde_yaml::Value::String(k.clone()),
                    model_to_value(item, schema.and_then(|node| node.property(k))),
                );
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

/// A short noun for a YAML value's shape, for error messages.
fn shape(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "a boolean",
        serde_yaml::Value::Number(_) => "a number",
        serde_yaml::Value::String(_) => "a string",
        serde_yaml::Value::Sequence(_) => "a sequence",
        serde_yaml::Value::Mapping(_) => "a mapping",
        serde_yaml::Value::Tagged(_) => "a tagged value",
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use indexmap::IndexMap;

    use crate::{
        Error, Match, Source, State, StateMetadata, Value, from_yaml, to_yaml_policy, to_yaml_query,
    };

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn parse_cidr(cidr: &str) -> (IpAddr, u8) {
        let (a, p) = cidr.split_once('/').unwrap();
        (a.parse().unwrap(), p.parse().unwrap())
    }

    /// Build the nested `ipv4: { addresses: [ { ip: <cidr> }, ... ] }` value.
    fn ipv4_with_addresses(cidrs: &[&str]) -> Value {
        let mut addr_list = Vec::new();
        for cidr in cidrs {
            let mut obj = IndexMap::new();
            obj.insert("ip".to_string(), Value::IpNetwork(parse_cidr(cidr)));
            addr_list.push(Value::Map(obj));
        }
        let mut ipv4 = IndexMap::new();
        ipv4.insert("addresses".to_string(), Value::List(addr_list));
        Value::Map(ipv4)
    }

    fn st(device_type: &str, match_spec: Match, fields: IndexMap<String, Value>) -> State {
        State {
            device_type: device_type.to_string(),
            match_spec,
            fields,
            source: Source::Static {
                policy: String::new(),
            },
            priority: 100,
            metadata: StateMetadata::new(),
        }
    }

    fn match_named(name: &str) -> Match {
        Match {
            name: Some(name.to_string()),
            ..Match::default()
        }
    }

    // ---------------------------------------------------------------- parse

    #[test]
    fn match_extracted_remaining_keys_are_fields() {
        let input =
            "match:\n  name: eth0\nmtu: 9000\nipv4:\n  addresses:\n    - ip: 10.0.1.50/24\n";
        let s = &from_yaml(input).unwrap()[0];
        assert_eq!(s.match_spec.name.as_deref(), Some("eth0"));
        assert!(!s.fields.contains_key("match"));
        assert_eq!(s.fields.get("mtu"), Some(&Value::U64(9000)));
        // The nested `ipv4` group is preserved, with the address as a network.
        assert_eq!(
            s.fields.get("ipv4"),
            Some(&ipv4_with_addresses(&["10.0.1.50/24"]))
        );
    }

    #[test]
    fn unknown_cidr_like_field_remains_string() {
        let s = &from_yaml("match:\n  name: eth0\nsubnet: 10.0.1.0/24").unwrap()[0];
        assert_eq!(
            s.fields.get("subnet"),
            Some(&Value::String("10.0.1.0/24".to_string()))
        );
    }

    #[test]
    fn top_level_list_yields_states_in_order() {
        let input = "- name: eth0\n  mtu: 9000\n- name: eth1\n  mtu: 1500\n";
        let states = from_yaml(input).unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(
            states[0].fields.get("name"),
            Some(&Value::String("eth0".to_string()))
        );
        assert_eq!(states[0].fields.get("mtu"), Some(&Value::U64(9000)));
        assert_eq!(
            states[1].fields.get("name"),
            Some(&Value::String("eth1".to_string()))
        );
        assert_eq!(states[1].fields.get("mtu"), Some(&Value::U64(1500)));
    }

    #[test]
    fn address_list_order_preserved() {
        let input = "match:\n  name: eth0\nipv4:\n  addresses:\n    - ip: 10.0.1.2/24\n    - ip: 10.0.1.1/24\n";
        let states = from_yaml(input).unwrap();
        let s = &states[0];
        let Value::Map(ipv4) = s.fields.get("ipv4").unwrap() else {
            panic!("ipv4 is not a map");
        };
        let Value::List(addrs) = ipv4.get("addresses").unwrap() else {
            panic!("addresses is not a list");
        };
        assert_eq!(addrs.len(), 2);
        // Element order is preserved: first address is 10.0.1.2/24.
        let Value::Map(a0) = &addrs[0] else { panic!() };
        assert_eq!(a0.get("ip"), Some(&Value::IpNetwork((ip("10.0.1.2"), 24))));
        let Value::Map(a1) = &addrs[1] else { panic!() };
        assert_eq!(a1.get("ip"), Some(&Value::IpNetwork((ip("10.0.1.1"), 24))));
        // Order also survives a re-serialize / re-parse round trip.
        let text = to_yaml_policy(&states).unwrap();
        let reparsed = from_yaml(&text).unwrap();
        assert!(states[0].content_eq(&reparsed[0]));
    }

    // YAML scalar-kind mapping. Semantic formats are selected by schema path.

    #[test]
    fn bool_lowercase() {
        let s = &from_yaml("x: true\ny: false").unwrap()[0];
        assert_eq!(s.fields.get("x"), Some(&Value::Bool(true)));
        assert_eq!(s.fields.get("y"), Some(&Value::Bool(false)));
    }

    #[test]
    fn bool_capitalized_spellings() {
        // Companion to the lib.rs doc: serde_yaml 0.9 resolves the true/false
        // spellings in any case.
        let s = &from_yaml("a: True\nb: TRUE\nc: FALSE").unwrap()[0];
        assert_eq!(s.fields.get("a"), Some(&Value::Bool(true)));
        assert_eq!(s.fields.get("b"), Some(&Value::Bool(true)));
        assert_eq!(s.fields.get("c"), Some(&Value::Bool(false)));
    }

    #[test]
    fn non_negative_int_u64() {
        let s = &from_yaml("mtu: 9000").unwrap()[0];
        assert_eq!(s.fields.get("mtu"), Some(&Value::U64(9000)));
    }

    #[test]
    fn negative_int_i64() {
        let s = &from_yaml("x: -5").unwrap()[0];
        assert_eq!(s.fields.get("x"), Some(&Value::I64(-5)));
    }

    #[test]
    fn cidr_like_unknown_values_remain_strings() {
        let s = &from_yaml("v4: 10.0.1.50/24\nv6: 2001:db8::/32").unwrap()[0];
        assert_eq!(
            s.fields.get("v4"),
            Some(&Value::String("10.0.1.50/24".to_string()))
        );
        assert_eq!(
            s.fields.get("v6"),
            Some(&Value::String("2001:db8::/32".to_string()))
        );
    }

    #[test]
    fn bare_ip_like_unknown_values_remain_strings() {
        let s = &from_yaml("v4: 10.0.1.5\nv6: 2001:db8::1").unwrap()[0];
        assert_eq!(
            s.fields.get("v4"),
            Some(&Value::String("10.0.1.5".to_string()))
        );
        assert_eq!(
            s.fields.get("v6"),
            Some(&Value::String("2001:db8::1".to_string()))
        );
    }

    #[test]
    fn plain_word_and_yes_are_strings() {
        let s = &from_yaml("word: hello\ny: yes").unwrap()[0];
        assert_eq!(
            s.fields.get("word"),
            Some(&Value::String("hello".to_string()))
        );
        assert_eq!(s.fields.get("y"), Some(&Value::String("yes".to_string())));
    }

    #[test]
    fn float_is_unsupported_scalar() {
        let err = from_yaml("x: 0.5").unwrap_err();
        assert_eq!(
            err,
            Error::UnsupportedScalar {
                kind: "float".to_string()
            }
        );
    }

    #[test]
    fn null_is_unsupported_scalar() {
        for input in ["x: ~", "x: null"] {
            let err = from_yaml(input).unwrap_err();
            assert_eq!(
                err,
                Error::UnsupportedScalar {
                    kind: "null".to_string()
                }
            );
        }
    }

    #[test]
    fn nested_list_and_map() {
        let input = "outer:\n  - 1\n  - true\ninner:\n  a: 1\n  b: true\n";
        let s = &from_yaml(input).unwrap()[0];
        assert_eq!(
            s.fields.get("outer"),
            Some(&Value::List(vec![Value::U64(1), Value::Bool(true)]))
        );
        let mut inner = IndexMap::new();
        inner.insert("a".to_string(), Value::U64(1));
        inner.insert("b".to_string(), Value::Bool(true));
        assert_eq!(s.fields.get("inner"), Some(&Value::Map(inner)));
        // Nested key order is preserved (positional).
        let Value::Map(inner_map) = s.fields.get("inner").unwrap() else {
            panic!()
        };
        let keys: Vec<&str> = inner_map.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn cidr_validity_is_checked_only_by_a_declared_format() {
        let s = &from_yaml("x: 10.0.1.0/33").unwrap()[0];
        assert_eq!(
            s.fields.get("x"),
            Some(&Value::String("10.0.1.0/33".to_string()))
        );
    }

    // `kind` handling.

    #[test]
    fn kind_absent_and_state_accepted_and_consumed() {
        for input in ["name: eth0", "kind: state\nname: eth0"] {
            let states = from_yaml(input).unwrap();
            assert_eq!(states.len(), 1);
            assert!(!states[0].fields.contains_key("kind"));
        }
    }

    #[test]
    fn kind_policy_is_error() {
        let err = from_yaml("kind: policy\nname: eth0").unwrap_err();
        assert_eq!(err, Error::PolicyWrapperNotSupported);
    }

    #[test]
    fn kind_unknown_is_error() {
        let err = from_yaml("kind: banana\nname: eth0").unwrap_err();
        assert_eq!(err, Error::UnknownKind("banana".to_string()));
    }

    // Document shape.

    #[test]
    fn empty_input_is_error() {
        for input in ["", "   ", "\n  \n"] {
            let err = from_yaml(input).unwrap_err();
            assert_eq!(err, Error::EmptyInput);
        }
    }

    #[test]
    fn explicit_empty_list_yields_zero_states() {
        let states = from_yaml("[]").unwrap();
        assert!(states.is_empty());
    }

    #[test]
    fn top_level_scalar_is_error() {
        assert!(matches!(
            from_yaml("just a string"),
            Err(Error::InvalidTopLevel(_))
        ));
        assert!(matches!(from_yaml("42"), Err(Error::InvalidTopLevel(_))));
    }

    #[test]
    fn non_mapping_list_element_is_error() {
        let err = from_yaml("- name: eth0\n- 42").unwrap_err();
        // A non-mapping list element is reported, carrying the element index
        // (asserted by variant, not by the message text).
        assert!(matches!(err, Error::InvalidTopLevel(_)));
    }

    #[test]
    fn leading_document_marker_accepted() {
        let states = from_yaml("---\nname: eth0").unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(
            states[0].fields.get("name"),
            Some(&Value::String("eth0".to_string()))
        );
    }

    #[test]
    fn second_document_is_multidocument_error() {
        // Assert the variant, never the upstream message text.
        let err = from_yaml("name: eth0\n---\nname: eth1").unwrap_err();
        assert_eq!(err, Error::MultiDocument);
    }

    // `type` routing.

    #[test]
    fn explicit_type_sets_device_type() {
        let s = &from_yaml("type: ethernet\nmtu: 9000").unwrap()[0];
        assert_eq!(s.device_type, "ethernet");
    }

    #[test]
    fn explicit_type_wins_over_tech_subobject() {
        let s = &from_yaml("type: wifi\nethernet:\n  speed: 1000").unwrap()[0];
        assert_eq!(s.device_type, "wifi");
    }

    #[test]
    fn registered_ethernet_subobject_implies_type() {
        let s = &from_yaml("ethernet:\n  speed: 1000").unwrap()[0];
        assert_eq!(s.device_type, "ethernet");
        let s = &from_yaml("wifi:\n  ssid: net").unwrap()[0];
        assert!(s.device_type.is_empty());
    }

    #[test]
    fn ipv4_subobject_does_not_imply_type() {
        let s = &from_yaml("ipv4:\n  addresses:\n    - ip: 10.0.1.50/24").unwrap()[0];
        assert!(s.device_type.is_empty());
    }

    #[test]
    fn non_string_type_value_is_error() {
        let err = from_yaml("type: 42").unwrap_err();
        assert_eq!(
            err,
            Error::UnsupportedScalar {
                kind: "a number".to_string()
            }
        );
    }

    // `match:` strictness.

    #[test]
    fn unknown_match_key_is_error() {
        let err = from_yaml("match:\n  name: eth0\n  bogus: x").unwrap_err();
        assert_eq!(err, Error::UnknownMatchKey("bogus".to_string()));
    }

    #[test]
    fn match_not_mapping_is_error() {
        let err = from_yaml("match: eth0").unwrap_err();
        assert_eq!(err, Error::MatchNotMapping);
    }

    #[test]
    fn non_string_match_value_is_error() {
        let err = from_yaml("match:\n  name: 42").unwrap_err();
        assert_eq!(
            err,
            Error::MatchValueNotString {
                key: "name".to_string()
            }
        );
    }

    // Runtime-field defaults on parsed states.

    #[test]
    fn parsed_states_get_static_source_priority_and_fresh_metadata() {
        let states = from_yaml("- name: eth0\n- name: eth1").unwrap();
        for s in &states {
            assert_eq!(
                s.source,
                Source::Static {
                    policy: String::new()
                }
            );
            assert_eq!(s.priority, 100);
            assert_eq!(s.metadata.id.get_version_num(), 7);
        }
        // Fresh metadata on every deserialization.
        let again = from_yaml("- name: eth0").unwrap();
        assert_ne!(states[0].metadata.id, again[0].metadata.id);
    }

    // Coexistence without merge or dedup.

    #[test]
    fn two_contributions_to_same_device_coexist_in_order() {
        let input = "- match:\n    name: eth0\n  mtu: 9000\n- match:\n    name: eth0\n  ipv4:\n    addresses:\n      - ip: 10.0.1.50/24\n";
        let states = from_yaml(input).unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].match_spec.name.as_deref(), Some("eth0"));
        assert_eq!(states[1].match_spec.name.as_deref(), Some("eth0"));
        // Both retained, in insertion order, no deduplication or rejection.
        assert_eq!(states[0].fields.get("mtu"), Some(&Value::U64(9000)));
        assert!(!states[0].fields.contains_key("ipv4"));
        assert!(states[1].fields.contains_key("ipv4"));
    }

    #[test]
    fn numeric_scalar_key_canonicalized() {
        let s = &from_yaml("9000: x").unwrap()[0];
        assert!(s.fields.contains_key("9000"));
        assert_eq!(s.fields.get("9000"), Some(&Value::String("x".to_string())));
    }

    // --------------------------------------------- serialize and round-trip

    #[test]
    fn reserved_top_level_fields_are_rejected() {
        for key in ["match", "type", "kind"] {
            let mut fields = IndexMap::new();
            fields.insert(key.to_string(), Value::String("value".to_string()));
            let state = st("ethernet", match_named("eth0"), fields);
            for render in [
                to_yaml_query as fn(&[State]) -> Result<String, Error>,
                to_yaml_policy,
            ] {
                assert_eq!(
                    render(std::slice::from_ref(&state)).unwrap_err(),
                    Error::ReservedFieldKey {
                        key: key.to_string(),
                        state_index: 0,
                    }
                );
            }
        }
    }

    #[test]
    fn reserved_top_level_field_reports_its_list_index() {
        let first = st("", Match::default(), IndexMap::new());
        let mut fields = IndexMap::new();
        fields.insert("type".to_string(), Value::String("ethernet".to_string()));
        let second = st("", Match::default(), fields);

        assert_eq!(
            to_yaml_query(&[first, second]).unwrap_err(),
            Error::ReservedFieldKey {
                key: "type".to_string(),
                state_index: 1,
            }
        );
    }

    #[test]
    fn reserved_names_in_nested_maps_are_allowed() {
        let mut nested = IndexMap::new();
        for key in ["match", "type", "kind"] {
            nested.insert(key.to_string(), Value::String("value".to_string()));
        }
        let mut fields = IndexMap::new();
        fields.insert("nested".to_string(), Value::Map(nested));
        let state = st("", Match::default(), fields);

        for render in [
            to_yaml_query as fn(&[State]) -> Result<String, Error>,
            to_yaml_policy,
        ] {
            assert!(render(std::slice::from_ref(&state)).is_ok());
        }
    }

    #[test]
    fn ordinary_fields_serialize_unchanged() {
        let mut fields = IndexMap::new();
        fields.insert("mtu".to_string(), Value::U64(9000));
        let state = st("ethernet", match_named("eth0"), fields);

        assert_eq!(
            to_yaml_query(std::slice::from_ref(&state)).unwrap(),
            "type: ethernet\nmtu: 9000\n"
        );
        assert_eq!(
            to_yaml_policy(&[state]).unwrap(),
            "match:\n  name: eth0\ntype: ethernet\nmtu: 9000\n"
        );
    }

    #[test]
    fn policy_roundtrip_preserves_content_and_order() {
        let mut fields = IndexMap::new();
        fields.insert("mtu".to_string(), Value::U64(9000));
        fields.insert("ipv4".to_string(), ipv4_with_addresses(&["10.0.1.50/24"]));
        fields.insert("enabled".to_string(), Value::Bool(true));
        let original = st("ethernet", match_named("eth0"), fields);
        let text = to_yaml_policy(std::slice::from_ref(&original)).unwrap();
        let reparsed = from_yaml(&text).unwrap();
        assert_eq!(reparsed.len(), 1);
        assert!(original.content_eq(&reparsed[0]));
        // Field key order asserted positionally.
        let keys: Vec<&str> = reparsed[0].fields.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["mtu", "ipv4", "enabled"]);
        // The address list's element order is preserved.
        let Value::Map(ipv4) = reparsed[0].fields.get("ipv4").unwrap() else {
            panic!()
        };
        let Value::List(addrs) = ipv4.get("addresses").unwrap() else {
            panic!()
        };
        assert_eq!(addrs.len(), 1);
        // Runtime fields reset: Static/100, fresh metadata.
        assert_eq!(
            reparsed[0].source,
            Source::Static {
                policy: String::new()
            }
        );
        assert_eq!(reparsed[0].priority, 100);
        assert_eq!(reparsed[0].metadata.id.get_version_num(), 7);
        assert_ne!(reparsed[0].metadata.id, original.metadata.id);
    }

    #[test]
    fn query_roundtrip_preserves_content() {
        let mut fields = IndexMap::new();
        fields.insert("name".to_string(), Value::String("eth0".to_string()));
        fields.insert(
            "mac".to_string(),
            Value::String("aa:bb:cc:dd:ee:ff".to_string()),
        );
        fields.insert("driver".to_string(), Value::String("ixgbe".to_string()));
        fields.insert("carrier".to_string(), Value::Bool(true));
        fields.insert("mtu".to_string(), Value::U64(9000));
        let base = st("ethernet", Match::default(), fields);
        // Simulate kernel-shaped actual state: Source::Kernel, priority 0.
        let kernel_state = State {
            source: Source::Kernel,
            priority: 0,
            ..base
        };
        let text = to_yaml_query(std::slice::from_ref(&kernel_state)).unwrap();
        let reparsed = from_yaml(&text).unwrap();
        assert!(kernel_state.content_eq(&reparsed[0]));
        assert_eq!(
            reparsed[0].source,
            Source::Static {
                policy: String::new()
            }
        );
        assert_eq!(reparsed[0].priority, 100);
        assert_ne!(reparsed[0].metadata.id, kernel_state.metadata.id);
    }

    #[test]
    fn query_format_omits_match_section() {
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        let original = st("ethernet", match_named("eth0"), f);
        let text = to_yaml_query(&[original]).unwrap();
        assert!(!text.contains("match:"));
        let reparsed = from_yaml(&text).unwrap();
        assert!(reparsed[0].match_spec.is_empty());
        assert_eq!(reparsed[0].device_type, "ethernet");
        assert_eq!(reparsed[0].fields.get("mtu"), Some(&Value::U64(9000)));
    }

    #[test]
    fn policy_format_match_section_fixed_key_order() {
        let m = Match {
            name: Some("eth0".to_string()),
            r#type: None,
            driver: Some("ixgbe".to_string()),
            pci_path: None,
            mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
        };
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        let original = st("", m, f);
        let text = to_yaml_policy(&[original]).unwrap();
        // The match sub-keys appear in the fixed order with None fields
        // omitted (checked via the textual position of the keys present).
        let name_pos = text.find("name:").unwrap();
        let driver_pos = text.find("driver:").unwrap();
        let mac_pos = text.find("mac:").unwrap();
        assert!(name_pos < driver_pos);
        assert!(driver_pos < mac_pos);
        assert!(!text.contains("pci_path"));
        let reparsed = from_yaml(&text).unwrap()[0].clone();
        assert_eq!(reparsed.match_spec.name.as_deref(), Some("eth0"));
        assert_eq!(reparsed.match_spec.driver.as_deref(), Some("ixgbe"));
        assert_eq!(
            reparsed.match_spec.mac.as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
        assert_eq!(reparsed.match_spec.r#type, None);
        assert_eq!(reparsed.match_spec.pci_path, None);
    }

    #[test]
    fn policy_format_keeps_an_empty_match_mapping() {
        let state = st("", Match::default(), IndexMap::new());
        assert_eq!(to_yaml_policy(&[state]).unwrap(), "match: {}\n");
    }

    #[test]
    fn one_state_is_mapping_two_states_are_sequence() {
        let mut f1 = IndexMap::new();
        f1.insert("name".to_string(), Value::String("eth0".to_string()));
        let mut f2 = IndexMap::new();
        f2.insert("name".to_string(), Value::String("eth1".to_string()));
        let s1 = st("", Match::default(), f1);
        let s2 = st("", Match::default(), f2);

        // One state -> top-level mapping (re-parses to exactly one state).
        let one = to_yaml_query(std::slice::from_ref(&s1)).unwrap();
        assert!(!one.trim_start().starts_with('-'));
        let one_reparsed = from_yaml(&one).unwrap();
        assert_eq!(one_reparsed.len(), 1);

        // Two states -> top-level sequence (re-parses to two, in order).
        let two = to_yaml_query(&[s1.clone(), s2.clone()]).unwrap();
        assert!(two.trim_start().starts_with('-'));
        let two_reparsed = from_yaml(&two).unwrap();
        assert_eq!(two_reparsed.len(), 2);
        assert!(s1.content_eq(&two_reparsed[0]));
        assert!(s2.content_eq(&two_reparsed[1]));
    }

    #[test]
    fn reserialization_is_idempotent() {
        // Single state with nested list/map, through the policy format.
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        f.insert("enabled".to_string(), Value::Bool(true));
        f.insert(
            "ipv4".to_string(),
            ipv4_with_addresses(&["10.0.1.50/24", "10.0.1.51/24"]),
        );
        let s1 = st("ethernet", match_named("eth0"), f);
        let a = to_yaml_policy(std::slice::from_ref(&s1)).unwrap();
        let b = to_yaml_policy(&[from_yaml(&a).unwrap()[0].clone()]).unwrap();
        let c = to_yaml_policy(&[from_yaml(&b).unwrap()[0].clone()]).unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);

        // Two-state list through the query format.
        let mut f2 = IndexMap::new();
        f2.insert("mtu".to_string(), Value::U64(1500));
        let s2 = st("", Match::default(), f2);
        let qa = to_yaml_query(&[s1.clone(), s2.clone()]).unwrap();
        let qb = to_yaml_query(&from_yaml(&qa).unwrap()).unwrap();
        let qc = to_yaml_query(&from_yaml(&qb).unwrap()).unwrap();
        assert_eq!(qa, qb);
        assert_eq!(qb, qc);
    }

    #[test]
    fn type_emitted_first_only_when_nonempty() {
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        // Non-empty device_type: type: precedes all fields.
        let with_type = st("ethernet", Match::default(), f.clone());
        let t = to_yaml_query(&[with_type]).unwrap();
        assert!(t.trim_start().starts_with("type:"));
        assert!(t.find("type:").unwrap() < t.find("mtu:").unwrap());
        // Empty device_type: no type: key at all.
        let no_type = st("", Match::default(), f);
        let t2 = to_yaml_query(&[no_type]).unwrap();
        assert!(!t2.contains("type:"));
    }

    #[test]
    fn policy_omits_an_implied_type_that_query_still_states() {
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        let mut ethernet = IndexMap::new();
        ethernet.insert("speed".to_string(), Value::U64(1000));
        f.insert("ethernet".to_string(), Value::Map(ethernet));
        let s = st("ethernet", match_named("eth0"), f);

        // Query output is a full report of the device, so it states the type
        // even though the `ethernet:` sub-object implies it.
        let query = to_yaml_query(std::slice::from_ref(&s)).unwrap();
        assert!(query.contains("type: ethernet"), "query was:\n{query}");

        // The policy format leaves it out: the sub-object carries it, and a
        // policy constrains the type through `match:`.
        let policy = to_yaml_policy(std::slice::from_ref(&s)).unwrap();
        assert!(
            !policy.contains("\ntype: ethernet"),
            "policy restated an implied type:\n{policy}"
        );
        // Omitting it is lossless: re-parsing re-derives it.
        let reparsed = from_yaml(&policy).unwrap();
        assert_eq!(reparsed[0].device_type, "ethernet");
        assert!(s.content_eq(&reparsed[0]));

        // A type the sub-object does not imply is still stated.
        let mut f2 = IndexMap::new();
        f2.insert("mtu".to_string(), Value::U64(9000));
        let bridge = st("bridge", match_named("br0"), f2);
        let policy = to_yaml_policy(&[bridge]).unwrap();
        assert!(policy.contains("type: bridge"), "policy was:\n{policy}");
    }

    #[test]
    fn no_reserved_or_runtime_keys_in_output() {
        let mut f = IndexMap::new();
        f.insert("mtu".to_string(), Value::U64(9000));
        let s = st("ethernet", match_named("eth0"), f);
        for text in [
            to_yaml_query(std::slice::from_ref(&s)).unwrap(),
            to_yaml_policy(std::slice::from_ref(&s)).unwrap(),
        ] {
            assert!(!text.contains("kind:"));
            assert!(!text.contains("source:"));
            assert!(!text.contains("priority:"));
        }
    }

    #[test]
    fn empty_slice_roundtrips_through_empty_list() {
        let text = to_yaml_query(&[]).unwrap();
        assert_eq!(text.trim(), "[]");
        assert!(from_yaml(&text).unwrap().is_empty());
    }

    #[test]
    fn string_quoting_corpus_roundtrips() {
        // Each of these, stored as a Value::String, must serialize and
        // re-parse to the identical Value::String (the renderer quotes the
        // plain-hostile ones so they do not re-resolve to a non-string).
        let corpus = [
            "true",
            "123",
            "null",
            "~",
            "",
            "a: b",
            "#hash",
            "line1\nline2",
            "10.0.1.5",
            "2001:db8::1",
            "10.0.1.50/24",
            "2001:db8::1/64",
        ];
        for value in corpus {
            let mut f = IndexMap::new();
            f.insert("s".to_string(), Value::String(value.to_string()));
            let s = st("", Match::default(), f);
            let text = to_yaml_query(&[s]).unwrap();
            let reparsed = from_yaml(&text).unwrap()[0].clone();
            assert_eq!(
                reparsed.fields.get("s"),
                Some(&Value::String(value.to_string())),
                "round-trip failed for {value:?}"
            );
        }
    }

    #[test]
    fn ip_like_string_roundtrips_without_retyping() {
        let mut f = IndexMap::new();
        f.insert("ip".to_string(), Value::String("10.0.1.5".to_string()));
        let s = st("", Match::default(), f);
        let text = to_yaml_query(&[s]).unwrap();
        let reparsed = from_yaml(&text).unwrap()[0].clone();
        assert_eq!(
            reparsed.fields.get("ip"),
            Some(&Value::String("10.0.1.5".to_string()))
        );
    }

    #[test]
    fn virtual_device_type_roundtrips() {
        // A virtual device type (here "bridge") round-trips with no
        // special-casing.
        let mut f = IndexMap::new();
        f.insert("name".to_string(), Value::String("br0".to_string()));
        let s = st("bridge", match_named("br0"), f);
        let text = to_yaml_policy(std::slice::from_ref(&s)).unwrap();
        let reparsed = from_yaml(&text).unwrap()[0].clone();
        assert!(s.content_eq(&reparsed));
        assert_eq!(reparsed.device_type, "bridge");
    }

    #[test]
    fn ip_network_requires_a_formatted_schema_path_on_reparse() {
        let mut f = IndexMap::new();
        f.insert(
            "subnet".to_string(),
            Value::IpNetwork((ip("10.0.1.50"), 24)),
        );
        let s = st("", Match::default(), f);
        let text = to_yaml_query(&[s]).unwrap();
        assert!(text.contains("10.0.1.50/24"), "text was: {text}");
        let reparsed = from_yaml(&text).unwrap()[0].clone();
        assert_eq!(
            reparsed.fields.get("subnet"),
            Some(&Value::String("10.0.1.50/24".to_string()))
        );
    }
}
