//! Tailchat 登录状态响应数据结构。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::data_struct::tailchat::UserId;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoginData {
    pub jwt: String,
    #[serde(rename = "userId")]
    pub user_id: UserId,
    pub email: String,
    pub nickname: String,
    pub avatar: String,
}

impl LoginData {
    /// 更新 `to_global` 状态。
    pub fn update_to_global(&self) {
        let status = crate::status::tailchat::MainStatus {
            enable: true,
            login: true,
            user_id: self.user_id.clone(),
            nick_name: self.nickname.clone(),
            email: self.email.clone(),
            jwt_token: self.jwt.clone(),
            avatar: self.avatar.clone(),
        };
        crate::MainStatus::update_tailchat_status(status);
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateDMConverse {
    /// 会话ID
    #[serde(rename = "_id")]
    pub id: String,
    /// 创建时间
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// 成员
    pub members: Vec<UserId>,
    /// 类型
    #[serde(rename = "type")]
    pub converse_type: String,
    /// 更新时间
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[allow(unused)]
pub type Writeable<T> = Arc<RwLock<T>>;

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct BotStatus {
    user_id: UserId,
}

#[allow(unused)]
impl BotStatus {
    /// 创建并初始化对应的数据结构。
    pub fn new(user_id: UserId) -> Self { Self { user_id } }

    /// 返回 `user_id` 对应的数据。
    pub fn get_user_id(&self) -> UserId { self.user_id.clone() }
}
