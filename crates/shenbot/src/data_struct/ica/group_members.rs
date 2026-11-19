//! Icalingua bridge 群成员信息。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value as JsonValue;

fn deserialize_string_or_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(value) = Option::<JsonValue>::deserialize(deserializer)? else {
        return Ok(String::new());
    };

    Ok(match value {
        JsonValue::Null => String::new(),
        JsonValue::String(value) => value,
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            serde_json::to_string(&value).unwrap_or_default()
        }
    })
}

fn deserialize_i64_or_default<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(value) = Option::<JsonValue>::deserialize(deserializer)? else {
        return Ok(0);
    };

    match value {
        JsonValue::Null => Ok(0),
        JsonValue::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| serde::de::Error::custom("integer is outside i64 range")),
        JsonValue::String(value) if value.trim().is_empty() => Ok(0),
        JsonValue::String(value) => value.parse().map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("expected integer, integer string, or null")),
    }
}

/// Bridge 返回的群成员资料。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GroupMember {
    #[serde(deserialize_with = "deserialize_i64_or_default")]
    pub user_id: i64,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub nickname: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub card: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub remark: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub level: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub role: String,
    #[serde(default, deserialize_with = "deserialize_i64_or_default")]
    pub shutup_time: i64,
}

impl GroupMember {
    /// 优先返回群名片，群名片为空时返回昵称。
    pub fn display_name(&self) -> &str {
        if self.card.trim().is_empty() {
            &self.nickname
        } else {
            &self.card
        }
    }

    /// 判断成员在给定 Unix 秒时间点是否仍处于禁言中。
    pub fn is_muted_at(&self, timestamp: i64) -> bool { self.shutup_time > timestamp }

    /// 判断成员当前是否仍处于禁言中。
    pub fn is_muted(&self) -> bool { self.is_muted_at(current_unix_timestamp()) }

    /// 返回给定 Unix 秒时间点的剩余禁言秒数。
    pub fn remaining_mute_seconds_at(&self, timestamp: i64) -> u64 {
        u64::try_from(self.shutup_time.saturating_sub(timestamp)).unwrap_or(0)
    }

    /// 返回当前剩余禁言秒数。
    pub fn remaining_mute_seconds(&self) -> u64 {
        self.remaining_mute_seconds_at(current_unix_timestamp())
    }
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::GroupMember;

    #[test]
    fn defaults_missing_fields_and_obeys_mute_boundary() {
        let member: GroupMember = serde_json::from_value(json!({
            "user_id": "123",
            "nickname": 456,
            "shutup_time": "100"
        }))
        .unwrap();

        assert_eq!(member.user_id, 123);
        assert_eq!(member.nickname, "456");
        assert_eq!(member.card, "");
        assert!(member.is_muted_at(99));
        assert!(!member.is_muted_at(100));
        assert_eq!(member.remaining_mute_seconds_at(98), 2);
    }
}
