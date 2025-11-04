use chrono::{TimeDelta, prelude::*};
use serde::Deserialize;
use std::collections::HashMap;
#[derive(Debug, Hash, PartialEq, PartialOrd, Eq)]
pub struct Key(String);

#[derive(Debug)]
pub struct KeycloakClient {
    pub url: String,
    pub client: reqwest::Client,
    pub cached_tokens: HashMap<Key, (String, DateTime<Utc>)>,
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

    /// returns a Bearer token
    /// returns a cached token valid at least one second
    /// if a cached token is available it will be returned
    pub async fn get_token(
        &mut self,
        realm: &str,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<String, reqwest::Error> {
        // check has cached token
        // check if token is valid
        // if not invalid entry and update
        let key = self.make_key(realm, client_id, username, password);
        if let Some((token, valid_until)) = self.cached_tokens.get(&key) {
            let now = chrono::Utc::now().timestamp();
            if valid_until.timestamp() - now > 1 {
                // at least one second remaining
                return Ok(token.clone());
            } else {
                println!("valid until {} and it is {}", valid_until.timestamp(), now);
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

        let resp = self.client.post(&token_url).form(&params).send().await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u32,
        }

        let token: TokenResponse = resp.json().await?;

        self.cached_tokens.insert(
            key,
            (
                token.access_token.clone(),
                chrono::Utc::now()
                    .checked_add_signed(TimeDelta::seconds(token.expires_in as i64))
                    .expect("failed to compute expiration date"),
            ),
        );

        Ok(token.access_token)
    }
}
