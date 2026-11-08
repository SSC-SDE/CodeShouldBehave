//! Open-PR numbers are scheduling hints, never comparison or source authority.
use super::*;

pub(crate) const DISCOVERY_PAGE_SIZE: usize = 20;

pub(crate) struct OpenPullPage {
    /// Includes unsupported forks for pagination/progress validation only.
    pub numbers: Vec<i64>,
    pub supported: Vec<i64>,
}

impl InstallationSession<'_> {
    /// Exactly two requests, plus the session's two credential requests. No Link
    /// URL is followed: only a numeric page on the fixed GitHub origin is used.
    pub(crate) fn open_pull_page(&self, page: i64) -> Result<OpenPullPage> {
        if page <= 0 || !slug(&self.owner) || !slug(&self.name) {
            return Err(SetupError::Invalid);
        }
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.credential.check(super::super::now()?)?;
        let prefix = format!("/repos/{}/{}", self.owner, self.name);
        let repo = self
            .broker
            .request("GET", &prefix, &self.credential.token, None)?;
        if repo["id"].as_i64() != Some(self.credential.repository)
            || repo["private"].as_bool() != Some(false)
            || repo["archived"].as_bool() != Some(false)
        {
            return Err(SetupError::Denied);
        }
        let value = self.broker.request(
            "GET",
            &format!("{prefix}/pulls?state=open&sort=created&direction=asc&per_page={DISCOVERY_PAGE_SIZE}&page={page}"),
            &self.credential.token,
            None,
        )?;
        let result = parse_page(&value, self.credential.repository)?;
        self.credential.check(super::super::now()?)?;
        Ok(result)
    }
}

fn parse_page(value: &Value, repository: i64) -> Result<OpenPullPage> {
    let rows = value.as_array().ok_or(SetupError::Invalid)?;
    if rows.len() > DISCOVERY_PAGE_SIZE {
        return Err(SetupError::Capacity);
    }
    let mut numbers = Vec::with_capacity(rows.len());
    let mut supported = Vec::with_capacity(rows.len());
    for row in rows {
        let number = row["number"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or(SetupError::Invalid)?;
        // GitHub PR numbers increase with creation order. Duplicate or reordered
        // rows cannot establish a valid page checkpoint.
        if numbers.last().is_some_and(|previous| *previous >= number)
            || row["state"].as_str() != Some("open")
            || row["draft"].as_bool().is_none()
            || row["base"]["repo"]["id"].as_i64() != Some(repository)
        {
            return Err(SetupError::Invalid);
        }
        numbers.push(number);
        let head = row
            .get("head")
            .and_then(|head| head.get("repo"))
            .ok_or(SetupError::Invalid)?;
        if head.is_null() {
            // Deleted fork: unsupported by the existing same-repository adapter.
            continue;
        }
        let head_id = head["id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or(SetupError::Invalid)?;
        let private = head["private"].as_bool().ok_or(SetupError::Invalid)?;
        if head_id == repository {
            if private {
                return Err(SetupError::Denied);
            }
            supported.push(number);
        }
    }
    Ok(OpenPullPage { numbers, supported })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pr(number: i64, head: Value) -> Value {
        json!({"number":number,"state":"open","draft":false,"base":{"repo":{"id":3}},"head":{"repo":head}})
    }
    #[test]
    fn inventory_rejects_malformed_duplicate_and_excessive_pages() {
        let row = pr(1, json!({"id":3,"private":false}));
        for value in [
            json!({"pulls":[]}),
            json!([row.clone(), row.clone()]),
            json!([{"number":1}]),
            json!(vec![row.clone(); 21]),
        ] {
            assert!(parse_page(&value, 3).is_err());
        }
        assert!(parse_page(&json!([pr(2, json!({"id":3,"private":false})), row]), 3).is_err());
    }
    #[test]
    fn forks_and_deleted_heads_never_expand_supported_semantics() {
        let page = parse_page(
            &json!([
                pr(1, json!({"id":3,"private":false})),
                pr(2, json!({"id":4,"private":false})),
                pr(3, Value::Null)
            ]),
            3,
        )
        .unwrap();
        assert_eq!(page.numbers, vec![1, 2, 3]);
        assert_eq!(page.supported, vec![1]);
        assert!(parse_page(&json!([pr(1, json!({"id":3,"private":true}))]), 3).is_err());
    }
    struct Fake(Mutex<Vec<String>>);
    impl Transport for std::sync::Arc<Fake> {
        fn request(
            &self,
            method: &str,
            path: &str,
            token: &str,
            body: Option<&Value>,
        ) -> Result<Value> {
            assert_eq!(method, "GET");
            assert_eq!(token, "test-only-repository-token");
            assert!(body.is_none());
            self.0.lock().unwrap().push(path.to_owned());
            match path {
                "/repos/fixture/app" => Ok(json!({"id":3,"private":false,"archived":false})),
                "/repos/fixture/app/pulls?state=open&sort=created&direction=asc&per_page=20&page=2" => {
                    Ok(json!([pr(21, json!({"id":3,"private":false}))]))
                }
                _ => panic!("unbounded or noncanonical inventory request: {path}"),
            }
        }
    }
    #[test]
    fn inventory_uses_exact_scoped_numeric_page_and_stops_before_unreserved_io() {
        let key = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .unwrap();
        assert!(key.status.success());
        let mut broker = GitHubInstallationBroker::new(1, "client123".into(), &key.stdout).unwrap();
        let fake = std::sync::Arc::new(Fake(Mutex::new(vec![])));
        broker.transport = Box::new(fake.clone());
        let session = InstallationSession {
            broker: &broker,
            credential: InstallationCredential {
                token: "test-only-repository-token".into(),
                installation: 2,
                repository: 3,
                expires: super::super::super::now().unwrap() + 60,
            },
            owner: "fixture".into(),
            name: "app".into(),
        };
        let pass = broker.begin_pass(1).unwrap();
        assert!(matches!(
            session.open_pull_page(2),
            Err(SetupError::Capacity)
        ));
        assert_eq!(fake.0.lock().unwrap().len(), 1);
        drop(pass);
        let _pass = broker.begin_pass(2).unwrap();
        let page = session.open_pull_page(2).unwrap();
        assert_eq!(page.supported, vec![21]);
        assert_eq!(fake.0.lock().unwrap().len(), 3);
        assert!(matches!(
            session.open_pull_page(0),
            Err(SetupError::Invalid)
        ));
        assert_eq!(fake.0.lock().unwrap().len(), 3);
    }
}
