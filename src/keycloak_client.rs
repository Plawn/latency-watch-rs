use serde::Deserialize;

#[derive(Debug)]
pub struct KeycloakClient {
    pub url: String,
}

impl KeycloakClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_owned(),
        }
    }

    pub async fn get_token(
        &self,
        realm: &str,
        client_id: &str,
        username: &str,
        password: &str,
    ) -> Result<String, reqwest::Error> {
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

        let client = reqwest::Client::new();
        let resp = client.post(&token_url).form(&params).send().await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let token: TokenResponse = resp.json().await?;
        Ok(token.access_token)
    }
}
