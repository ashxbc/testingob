use anyhow::Result;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::Serialize;

#[derive(Clone)]
pub struct RedisBus {
    pub conn: ConnectionManager,
}

impl RedisBus {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }

    pub async fn publish<T: Serialize>(&mut self, channel: &str, payload: &T) -> Result<()> {
        let json = serde_json::to_string(payload)?;
        let _: () = self.conn.publish(channel, json).await?;
        Ok(())
    }

    pub async fn set_json<T: Serialize>(&mut self, key: &str, payload: &T) -> Result<()> {
        let json = serde_json::to_string(payload)?;
        let _: () = self.conn.set(key, json).await?;
        Ok(())
    }

    pub async fn lpush_capped<T: Serialize>(
        &mut self,
        key: &str,
        payload: &T,
        cap: isize,
    ) -> Result<()> {
        let json = serde_json::to_string(payload)?;
        let _: () = self.conn.lpush(key, json).await?;
        let _: () = self.conn.ltrim(key, 0, cap - 1).await?;
        Ok(())
    }
}
