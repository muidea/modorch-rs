use modorch_core::{FrameworkError, FrameworkResult};

const MAX_TOPIC_BYTES: usize = 65_535;

fn validate_utf8_field(raw: &str) -> FrameworkResult<()> {
    // &str already guarantees well-formed UTF-8; MQTT lengths count encoded bytes.
    if raw.is_empty() || raw.len() > MAX_TOPIC_BYTES || raw.contains('\0') {
        return Err(FrameworkError::InvalidConfig(
            "MQTT topic must be nonempty, at most 65535 UTF-8 bytes and contain no NUL".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_name(raw: &str) -> FrameworkResult<()> {
    validate_utf8_field(raw)?;
    if raw.contains(['+', '#']) {
        return Err(FrameworkError::InvalidConfig(
            "MQTT Topic Name cannot contain + or #".into(),
        ));
    }
    Ok(())
}

enum Level {
    Literal(Box<str>),
    Single,
    Multi,
}

pub(crate) struct TopicFilter {
    levels: Vec<Level>,
}

impl TopicFilter {
    pub(crate) fn parse(raw: &str) -> FrameworkResult<Self> {
        validate_utf8_field(raw)?;
        let mut levels = Vec::new();
        let mut parts = raw.split('/').peekable();
        while let Some(part) = parts.next() {
            levels.push(match part {
                "+" => Level::Single,
                "#" if parts.peek().is_none() => Level::Multi,
                literal if !literal.contains(['+', '#']) => Level::Literal(literal.into()),
                _ => {
                    return Err(FrameworkError::InvalidConfig(
                        "MQTT Topic Filter requires whole-level + and final whole-level #".into(),
                    ));
                }
            });
        }
        Ok(Self { levels })
    }

    pub(crate) fn has_wildcards(&self) -> bool {
        self.levels
            .iter()
            .any(|level| !matches!(level, Level::Literal(_)))
    }

    // Caller validates the Topic Name once, before iterating all candidate filters.
    pub(crate) fn matches_validated(&self, topic: &str) -> bool {
        if topic.starts_with('$') && !matches!(self.levels[0], Level::Literal(_)) {
            return false;
        }
        let mut actual = topic.split('/');
        for level in &self.levels {
            match level {
                Level::Multi => return true,
                Level::Single => {
                    if actual.next().is_none() {
                        return false;
                    }
                }
                Level::Literal(value) => {
                    if actual.next() != Some(value.as_ref()) {
                        return false;
                    }
                }
            }
        }
        actual.next().is_none()
    }
}
