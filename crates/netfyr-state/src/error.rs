//! The crate-wide error type.

use std::fmt;

use crate::schema::ValidationError;

/// A schema-validation error together with its input-state index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedValidationError {
    /// The index of the invalid state in the apply input.
    pub state_index: usize,
    /// The validation error found for that state.
    pub error: ValidationError,
}

/// Errors from parsing, serializing, and apply-path preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A YAML error from the parser or renderer; the message keeps the
    /// library's line/column context.
    Yaml(String),
    /// The input started a second YAML document with a `---` separator.
    MultiDocument,
    /// The input was empty or whitespace-only.
    EmptyInput,
    /// The top level of the document, or an element of its top-level list,
    /// was not a device mapping (or list of them). Carries a description of
    /// the shape found.
    InvalidTopLevel(String),
    /// A `kind:` key whose value is neither `state` nor `policy`.
    UnknownKind(String),
    /// `kind: policy`: policy wrappers are defined and parsed by the
    /// policy spec, not this one.
    PolicyWrapperNotSupported,
    /// A `match:` section carried a key other than `name`, `type`,
    /// `driver`, `pci_path`, or `mac`.
    UnknownMatchKey(String),
    /// The `match:` section was not a mapping.
    MatchNotMapping,
    /// A mapping key was a null, sequence, mapping, or tagged value, which
    /// has no string form usable as a field or match key. Carries a
    /// description of the shape found.
    InvalidFieldKey(String),
    /// A `match:` field's value was not a string scalar.
    MatchValueNotString {
        /// The offending match field name.
        key: String,
    },
    /// Query-mode YAML must not carry an explicit non-empty `match:` section.
    UnexpectedMatch {
        /// The index of the offending state in the input list.
        state_index: usize,
    },
    /// Policy-mode YAML requires a non-empty selector.
    MissingMatch {
        /// The index of the offending state in the input list.
        state_index: usize,
    },
    /// A YAML scalar with no [`crate::Value`] variant (a float or null).
    UnsupportedScalar {
        /// A short description of the value's kind.
        kind: String,
    },
    /// Apply path: a state with an empty match lacks the field the match
    /// strategy needs.
    NoMatchField {
        /// The field the strategy looks for.
        field: String,
        /// The index of the offending state in the input list.
        state_index: usize,
    },
    /// A public state field conflicts with a YAML format key.
    ReservedFieldKey {
        /// The field name that collides with a reserved YAML key.
        key: String,
        /// The index of the offending state in the input list.
        state_index: usize,
    },
    /// Apply input or its prepared result failed schema validation.
    Validation(Vec<IndexedValidationError>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Yaml(message) => write!(f, "YAML error: {message}"),
            Error::MultiDocument => write!(
                f,
                "input contains more than one YAML document ('---' separator); \
                 netfyr's format expresses multiple devices as a top-level list"
            ),
            Error::EmptyInput => write!(f, "input is empty"),
            Error::InvalidTopLevel(shape) => write!(
                f,
                "invalid top level: expected a device mapping or a list of device mappings, \
                 found {shape}"
            ),
            Error::UnknownKind(kind) => {
                write!(
                    f,
                    "unknown kind '{kind}'; expected 'state' (or omit the key)"
                )
            }
            Error::PolicyWrapperNotSupported => write!(
                f,
                "kind 'policy' documents are not parsed here; policy wrappers are handled \
                 by the policy parser"
            ),
            Error::UnknownMatchKey(key) => write!(
                f,
                "unknown match field '{key}'; valid fields are: name, type, driver, pci_path, mac"
            ),
            Error::MatchNotMapping => write!(
                f,
                "the 'match' section must be a mapping of field names to string values"
            ),
            Error::MatchValueNotString { key } => write!(f, "match field '{key}' must be a string"),
            Error::UnexpectedMatch { state_index } => write!(
                f,
                "state at index {state_index} has a match section, which is not allowed in query YAML"
            ),
            Error::MissingMatch { state_index } => write!(
                f,
                "state at index {state_index} has no match section, which is required in policy YAML"
            ),
            Error::InvalidFieldKey(shape) => write!(
                f,
                "invalid field key (found {shape}); keys must be strings or numbers"
            ),
            Error::UnsupportedScalar { kind } => write!(
                f,
                "unsupported YAML value of type {kind}; quote the value to store it as a string"
            ),
            Error::NoMatchField { field, state_index } => write!(
                f,
                "state at index {state_index} has no '{field}' field, which is needed to \
                 generate its match"
            ),
            Error::ReservedFieldKey { key, state_index } => write!(
                f,
                "state at index {state_index} has field '{key}', but 'match', 'type', and \
                  'kind' are reserved top-level YAML keys"
            ),
            Error::Validation(errors) => {
                write!(f, "schema validation failed")?;
                for indexed in errors {
                    write!(f, "; state {}: {}", indexed.state_index, indexed.error)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}
