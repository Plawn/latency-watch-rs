use chrono::{TimeDelta, prelude::*};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Hash, PartialEq, PartialOrd, Eq)]
struct Key(String);

#[derive(Debug)]
pub struct KeycloakClient {
    url: String,
    client: reqwest::Client,
    cached_tokens: HashMap<Key, (String, DateTime<Utc>)>,
}

impl KeycloakClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            cached_tokens: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    fn make_key(&self, realm: &str, client_id: &str, username: &str, password: &str) -> Key {
        let s = format!("{}:{}:{}:{}", realm, client_id, username, password);
        Key(s)
    }

    /// Remove all expired tokens from the cache
    pub fn cleanup_expired_tokens(&mut self) {
        let now = Utc::now();
        self.cached_tokens.retain(|_, (_, valid_until)| *valid_until > now);
    }

    /// Returns a Bearer token.
    /// Returns a cached token if available and valid for at least one second.
    pub async fn get_token(
        &mut self,
        realm: &str,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<String, reqwest::Error> {
        // Cleanup expired tokens periodically
        self.cleanup_expired_tokens();

        let key = self.make_key(realm, client_id, username, password);
        if let Some((token, valid_until)) = self.cached_tokens.get(&key) {
            let now = chrono::Utc::now().timestamp();
            if valid_until.timestamp() - now > 1 {
                tracing::debug!(
                    realm = %realm,
                    client_id = %client_id,
                    "Using cached OAuth token"
                );
                return Ok(token.clone());
            } else {
                tracing::debug!(
                    valid_until = %valid_until.timestamp(),
                    now = %now,
                    "Token expired, refreshing"
                );
            }
        }

        let token_url = format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.url, realm
        );
        let params = [
            ("grant_type", "password"),
            ("client_id", client_id),
            ("username", username),
            ("password", password),
        ];

        tracing::debug!(realm = %realm, client_id = %client_id, "Fetching new OAuth token");

        let resp = self.client.post(&token_url).form(&params).send().await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u32,
        }

        let token: TokenResponse = resp.json().await?;

        let expires_at = chrono::Utc::now()
            .checked_add_signed(TimeDelta::seconds(token.expires_in as i64))
            .unwrap_or_else(|| {
                tracing::warn!("Failed to compute expiration, using 1 hour default");
                chrono::Utc::now() + TimeDelta::hours(1)
            });

        self.cached_tokens.insert(key, (token.access_token.clone(), expires_at));

        Ok(token.access_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_client() {
        let client = KeycloakClient::new("http://localhost:8080");
        assert_eq!(client.url, "http://localhost:8080");
        assert!(client.cached_tokens.is_empty());
    }

    #[test]
    fn test_make_key_format() {
        let client = KeycloakClient::new("http://test");
        let key = client.make_key("myrealm", "myclient", "user1", "pass123");
        assert_eq!(key.0, "myrealm:myclient:user1:pass123");
    }

    #[test]
    fn test_cleanup_expired_tokens() {
        let mut client = KeycloakClient::new("http://test");

        // Add an expired token
        let expired_key = Key("expired".to_string());
        let expired_time = Utc::now() - TimeDelta::hours(1);
        client.cached_tokens.insert(expired_key, ("old_token".to_string(), expired_time));

        // Add a valid token
        let valid_key = Key("valid".to_string());
        let valid_time = Utc::now() + TimeDelta::hours(1);
        client.cached_tokens.insert(valid_key, ("new_token".to_string(), valid_time));

        assert_eq!(client.cached_tokens.len(), 2);

        client.cleanup_expired_tokens();

        assert_eq!(client.cached_tokens.len(), 1);
        assert!(client.cached_tokens.contains_key(&Key("valid".to_string())));
    }
}
