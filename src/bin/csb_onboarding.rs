//! Local adapter entry point. Invoked only by the loopback host after gh access checks.
#[path = "csb_onboarding/database_copy.rs"]
mod database_copy;
#[path = "csb_onboarding/hld.rs"]
mod hld;
#[path = "csb_onboarding/lld.rs"]
mod lld;
use flowdiff::{
    Action, Grant, HttpBackend, HttpFailure, HttpOperation, LocalOnboardingSource,
    OnboardingAccessProvider, OnboardingHost, PrincipalId, RepositoryId, SecurityDomain,
    ServiceAuthenticator, Storage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::PathBuf,
    sync::Arc,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    principal: String,
    maintainer: bool,
    action: String,
    id: Option<String>,
    request: Value,
}
struct Auth(PrincipalId);
impl ServiceAuthenticator for Auth {
    fn authenticate(&self, _: &str) -> Result<PrincipalId, String> {
        Ok(self.0.clone())
    }
}
struct Access(Vec<Grant>);
impl OnboardingAccessProvider for Access {
    fn grants(&self, _: &PrincipalId, _: &SecurityDomain) -> Result<Vec<Grant>, String> {
        Ok(self.0.clone())
    }
}
struct NoReview;
impl HttpBackend for NoReview {
    fn execute(&self, _: &str, _: HttpOperation) -> Result<Value, HttpFailure> {
        Err(HttpFailure::DeniedOrMissing)
    }
}
fn run() -> Result<Value, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("Expected operator repository, state directory and GitHub slug".into());
    }
    let mut input = String::new();
    std::io::stdin()
        .take(65537)
        .read_to_string(&mut input)
        .map_err(|_| "Cannot read local request")?;
    if input.len() > 65536 {
        return Err("Local request exceeds 64 KiB".into());
    }
    let input: Invocation = serde_json::from_str(&input).map_err(|_| "Invalid local request")?;
    if input.action == "database-copy" {
        // Only the trusted local host supplies these operator paths. No HTTP route exists.
        return database_copy::invoke(input.request);
    }
    let (owner, name) = args[2]
        .split_once('/')
        .ok_or("Invalid repository identity")?;
    let repository =
        RepositoryId::new(SecurityDomain::new("local").unwrap(), "github", owner, name)?;
    let principal = PrincipalId::new(input.principal)?;
    let mut actions =
        BTreeSet::from([Action::Index, Action::Graph, Action::Source, Action::Export]);
    if input.maintainer {
        actions.insert(Action::ConfirmArchitecture);
    }
    let grants = vec![Grant {
        principal: principal.clone(),
        repository: repository.clone(),
        actions,
        paths: flowdiff::PathScope::all(),
    }];
    let root = PathBuf::from(&args[1]);
    database_copy::prepare_operator_database(&root)?;
    let mut storage = Storage::open(&root.join("onboarding.sqlite"), &root.join("artifacts"))?;
    if matches!(
        input.action.as_str(),
        "hld-view" | "hld-read" | "hld-preview" | "hld-accept"
    ) {
        let value = hld::invoke(
            &mut storage,
            &repository,
            &principal,
            &grants,
            &input.action,
            input.request,
        );
        return Ok(match value {
            Ok(value) => json!({"status":200,"value":value}),
            Err(error) if error.starts_with("Stale") => {
                json!({"status":409,"value":{"error":"stale_hld_preview"}})
            }
            Err(_) => json!({"status":404,"value":{"error":"hld_unavailable_in_current_context"}}),
        });
    }
    if input.action.starts_with("lld-") {
        return Ok(
            match lld::invoke(
                &mut storage,
                &repository,
                &principal,
                &grants,
                &input.action,
                input.request,
            ) {
                Ok(value) => json!({"status":200,"value":value}),
                Err(error) if error.starts_with("Stale") => {
                    json!({"status":409,"value":{"error":"stale_lld_preview"}})
                }
                Err(_) => {
                    json!({"status":404,"value":{"error":"lld_unavailable_in_current_context"}})
                }
            },
        );
    }
    let host = OnboardingHost::new(
        storage,
        Auth(principal),
        Access(grants),
        LocalOnboardingSource::new(BTreeMap::from([(repository, PathBuf::from(&args[0]))])),
        Arc::new(NoReview),
    );
    let operation = match input.action.as_str() {
        "create" => HttpOperation::CreateOnboarding {
            request: input.request,
        },
        "read" => HttpOperation::ReadOnboarding {
            id: input.id.ok_or("Missing session ID")?,
            domain: "local".into(),
        },
        "scan"
        | "detect"
        | "inspect"
        | "cancel"
        | "decisions"
        | "provider-decisions"
        | "integration-decisions"
        | "confirm"
        | "activate"
        | "defaults"
        | "drift" => HttpOperation::MutateOnboarding {
            id: input.id.ok_or("Missing session ID")?,
            action: input.action,
            request: input.request,
        },
        _ => return Err("Invalid onboarding operation".into()),
    };
    Ok(match host.execute("local-host", operation) {
        Ok(value) => json!({"status":200,"value":value}),
        Err(error) => {
            let status = match error {
                HttpFailure::Authentication => 401,
                HttpFailure::DeniedOrMissing => 404,
                HttpFailure::Conflict => 409,
                HttpFailure::TooLarge => 413,
                HttpFailure::Unavailable => 503,
                HttpFailure::Malformed => 400,
                HttpFailure::Internal => 500,
            };
            json!({"status":status,"value":{"api_version":"v1","error":format!("{error:?}")}})
        }
    })
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!(
                "{}",
                json!({"status":503,"value":{"error":if error == "local_database_copy_unavailable" {"local_database_copy_unavailable"} else {"local_onboarding_unavailable"}}})
            );
            std::process::exit(1);
        }
    }
}
