//! The error type of the world document, and the checks that produce it.

use crate::EntityId;
use std::fmt;

/// Everything that can be wrong with a world document or a request against it.
///
/// [`Display`](fmt::Display) gives a sentence an agent can act on: it names the field, the
/// value and what is allowed.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldError {
    /// A value is outside its allowed range or breaks a rule.
    Invalid {
        /// The entity the field belongs to, if any.
        entity: Option<EntityId>,
        /// The field, as a path (`terrain.cell_m`, `shape.size[0]`).
        field: String,
        /// The offending value, as text.
        value: String,
        /// What is allowed, phrased as the end of a sentence (`it must be between 1 and 10`).
        expected: String,
    },
    /// The text is not valid JSON or does not have the shape of a world document.
    Json {
        /// What the parser reported (it names unknown or missing fields and wrong types).
        message: String,
        /// One-based line of the problem (0 when unknown).
        line: usize,
        /// One-based column of the problem (0 when unknown).
        column: usize,
    },
    /// The `gamecraft` format version is missing or not one this engine reads.
    Version {
        /// The value found, as JSON text (`missing` when absent).
        found: String,
    },
    /// Two entities share an id.
    DuplicateEntityId {
        /// The repeated id.
        id: EntityId,
    },
    /// An entity id is not below `next_entity_id`, so it could be handed out again.
    EntityIdNotBelowNext {
        /// The entity's id.
        id: EntityId,
        /// The document's `next_entity_id`.
        next: EntityId,
    },
    /// An entity has two components of the same type.
    DuplicateComponent {
        /// The entity.
        entity: EntityId,
        /// The component type (`shape`, `plant`, `character`).
        kind: String,
    },
    /// The world would hold more entities than allowed.
    TooManyEntities {
        /// How many entities there are (or would be).
        count: usize,
        /// The limit, [`crate::limits::MAX_ENTITIES`].
        max: usize,
    },
    /// No entity id is left to hand out (`next_entity_id` reached `u64::MAX`).
    IdsExhausted,
    /// The terrain needs more height samples than allowed.
    TerrainTooLarge {
        /// The terrain side, metres.
        size_m: f64,
        /// The requested cell, metres.
        cell_m: f64,
        /// Samples per side the request needs.
        samples: u64,
        /// The limit, [`crate::limits::TERRAIN_MAX_SAMPLES_PER_SIDE`].
        max: usize,
        /// The smallest cell size that fits.
        fitting_cell_m: f64,
    },
    /// No entity matches a reference.
    NotFound {
        /// The reference as given.
        reference: String,
        /// Entities the caller may have meant, as `id "name"`.
        candidates: Vec<String>,
    },
    /// Several entities match a name.
    Ambiguous {
        /// The reference as given.
        reference: String,
        /// The matching entities, as `id "name"`.
        candidates: Vec<String>,
    },
    /// The document could not be written as JSON.
    Serialize {
        /// What the serialiser reported.
        message: String,
    },
    /// Something is wrong with a prefab.
    Prefab {
        /// The prefab's id.
        prefab: String,
        /// What is wrong, naming the field.
        message: String,
    },
    /// No prefab has this id.
    PrefabNotFound {
        /// The id as given.
        prefab: String,
        /// The ids of the world's prefabs (at most 10).
        known: Vec<String>,
    },
    /// A generator entry breaks a rule.
    Generator {
        /// The generator's id.
        generator: String,
        /// What is wrong and what is allowed, as a sentence.
        message: String,
    },
    /// A prefab cannot be removed while entities are placed from it.
    PrefabInUse {
        /// The prefab's id.
        prefab: String,
        /// Its instances, as `id "name"` (at most 10).
        entities: Vec<String>,
        /// How many instances there are.
        count: usize,
    },
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldError::Invalid {
                entity,
                field,
                value,
                expected,
            } => {
                if let Some(id) = entity {
                    write!(f, "entity {id}: ")?;
                }
                write!(f, "{field} is {value}; {expected}")
            }
            WorldError::Json {
                message,
                line,
                column,
            } => {
                write!(f, "the world JSON is not valid: {message}")?;
                if *line > 0 && !message.contains(" at line ") {
                    write!(f, " (line {line}, column {column})")?;
                }
                Ok(())
            }
            WorldError::Version { found } => write!(
                f,
                "gamecraft (the format version) is {found}; this engine reads version {} only",
                crate::FORMAT_VERSION
            ),
            WorldError::DuplicateEntityId { id } => write!(
                f,
                "entities: id {id} is used by more than one entity; every entity needs its own id"
            ),
            WorldError::EntityIdNotBelowNext { id, next } => write!(
                f,
                "entity {id}: id {id} is not below next_entity_id {next}; \
                 set next_entity_id to at least {}",
                id.0.saturating_add(1)
            ),
            WorldError::DuplicateComponent { entity, kind } => write!(
                f,
                "entity {entity}: components holds more than one `{kind}`; \
                 an entity has at most one component of each type"
            ),
            WorldError::TooManyEntities { count, max } => write!(
                f,
                "entities: the world would hold {count} entities; at most {max} are allowed"
            ),
            WorldError::IdsExhausted => write!(
                f,
                "next_entity_id has reached its maximum; no new entity id can be handed out"
            ),
            WorldError::TerrainTooLarge {
                size_m,
                cell_m,
                samples,
                max,
                fitting_cell_m,
            } => write!(
                f,
                "terrain.cell_m is {cell_m}: a terrain of size_m {size_m} then needs \
                 {samples} x {samples} height samples, but at most {max} x {max} are allowed; \
                 use cell_m {fitting_cell_m} or larger"
            ),
            WorldError::NotFound {
                reference,
                candidates,
            } => {
                write!(f, "no entity has the id or name \"{reference}\"")?;
                if candidates.is_empty() {
                    write!(f, "; the world has no entities")
                } else {
                    write!(f, "; candidates: {}", candidates.join(", "))
                }
            }
            WorldError::Ambiguous {
                reference,
                candidates,
            } => write!(
                f,
                "\"{reference}\" matches {} entities: {}; refer to one by its id",
                candidates.len(),
                candidates.join(", ")
            ),
            WorldError::Serialize { message } => {
                write!(f, "the world could not be written as JSON: {message}")
            }
            WorldError::Prefab { prefab, message } => write!(f, "prefab \"{prefab}\": {message}"),
            WorldError::PrefabNotFound { prefab, known } => {
                write!(f, "no prefab has the id \"{prefab}\"")?;
                if known.is_empty() {
                    write!(f, "; the world has no prefabs (add one with prefab.add)")
                } else {
                    write!(f, "; the prefabs are: {}", known.join(", "))
                }
            }
            WorldError::Generator { generator, message } => {
                write!(f, "generator \"{generator}\": {message}")
            }
            WorldError::PrefabInUse {
                prefab,
                entities,
                count,
            } => write!(
                f,
                "prefab \"{prefab}\" is placed as {count} entit{}: {}{}; remove them first",
                if *count == 1 { "y" } else { "ies" },
                entities.join(", "),
                if *count > entities.len() { ", ..." } else { "" }
            ),
        }
    }
}

impl std::error::Error for WorldError {}

impl From<serde_json::Error> for WorldError {
    fn from(e: serde_json::Error) -> Self {
        WorldError::Json {
            message: e.to_string(),
            line: e.line(),
            column: e.column(),
        }
    }
}

/// Where a checked field lives: the entity it belongs to (if any).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ctx {
    pub(crate) entity: Option<EntityId>,
}

impl Ctx {
    pub(crate) const WORLD: Ctx = Ctx { entity: None };

    pub(crate) fn entity(id: EntityId) -> Ctx {
        Ctx { entity: Some(id) }
    }

    /// An [`WorldError::Invalid`] for `field`.
    pub(crate) fn invalid(
        self,
        field: impl Into<String>,
        value: impl fmt::Display,
        expected: impl Into<String>,
    ) -> WorldError {
        WorldError::Invalid {
            entity: self.entity,
            field: field.into(),
            value: value.to_string(),
            expected: expected.into(),
        }
    }

    /// `value` must be finite and within `min..=max`.
    pub(crate) fn range(
        self,
        field: impl Into<String>,
        value: f64,
        min: f64,
        max: f64,
        unit: &str,
    ) -> Result<(), WorldError> {
        if value.is_finite() && value >= min && value <= max {
            Ok(())
        } else {
            Err(self.invalid(
                field,
                value,
                format!("it must be a finite number from {min} to {max}{unit}"),
            ))
        }
    }

    /// `value` must be finite and above 0.
    pub(crate) fn positive(
        self,
        field: impl Into<String>,
        value: f64,
        unit: &str,
    ) -> Result<(), WorldError> {
        if value.is_finite() && value > 0.0 {
            Ok(())
        } else {
            Err(self.invalid(
                field,
                value,
                format!("it must be a finite number above 0{unit}"),
            ))
        }
    }

    /// `value` must be finite.
    pub(crate) fn finite(self, field: impl Into<String>, value: f64) -> Result<(), WorldError> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(self.invalid(field, value, "it must be a finite number"))
        }
    }

    /// `text` must have at most `max` characters.
    pub(crate) fn text(
        self,
        field: impl Into<String>,
        text: &str,
        max: usize,
    ) -> Result<(), WorldError> {
        let chars = text.chars().count();
        if chars <= max {
            Ok(())
        } else {
            Err(self.invalid(
                field,
                format!("{chars} characters long"),
                format!("it must be at most {max} characters long"),
            ))
        }
    }

    /// `id` must not be empty (or only spaces) and have at most
    /// [`crate::limits::MAX_NAME_CHARS`] characters.
    pub(crate) fn id(self, field: impl Into<String>, id: &str) -> Result<(), WorldError> {
        let field = field.into();
        if id.trim().is_empty() {
            return Err(self.invalid(
                field,
                "empty",
                "it must hold at least one character other than a space",
            ));
        }
        self.name(field, id)
    }

    /// `name` must have at most [`crate::limits::MAX_NAME_CHARS`] characters.
    pub(crate) fn name(self, field: impl Into<String>, name: &str) -> Result<(), WorldError> {
        let chars = name.chars().count();
        if chars <= crate::limits::MAX_NAME_CHARS {
            Ok(())
        } else {
            Err(self.invalid(
                field,
                format!("{chars} characters long"),
                format!(
                    "it must be at most {} characters long",
                    crate::limits::MAX_NAME_CHARS
                ),
            ))
        }
    }
}
