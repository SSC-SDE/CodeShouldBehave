//! Local operator-only exporter. It never publishes files or contacts providers.
use flowdiff::public_report::{
    ExpectedComparison, PublicPolicy, PublicRepository, export_public_report, read_json_file,
    verify_bundle, write_bundle,
};
use std::{collections::BTreeMap, path::Path};
fn run() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|a| a == "verify") {
        if arguments.len() != 3 || arguments[1] != "--bundle" {
            return Err("Usage: csb-public-report verify --bundle DIR".into());
        }
        let manifest = verify_bundle(Path::new(&arguments[2]))?;
        println!(
            "{}",
            serde_json::to_string(&manifest).map_err(|_| "Cannot encode verified manifest")?
        );
        return Ok(());
    }
    let mut args = arguments.into_iter();
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        if key == "--help" {
            println!(
                "csb-public-report --artifact PRIVATE_JSON --policy BASE_OWNED_POLICY_JSON --output NEW_DIR --repository-id N --repository OWNER/NAME --pr-number N --selected-baseline SHA --baseline SHA --proposal SHA\nLocal export only. Current visibility, trusted policy origin and publication require separate CI validation."
            );
            return Ok(());
        }
        if ![
            "--artifact",
            "--policy",
            "--output",
            "--repository-id",
            "--repository",
            "--pr-number",
            "--selected-baseline",
            "--baseline",
            "--proposal",
        ]
        .contains(&key.as_str())
            || values.contains_key(&key)
        {
            return Err("Unknown/duplicate public export argument".into());
        }
        values.insert(key, args.next().ok_or("Missing argument value")?);
    }
    let get = |key: &str| {
        values
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("Missing {key}"))
    };
    let (owner, name) = get("--repository")?
        .split_once('/')
        .ok_or("Repository must be OWNER/NAME")?;
    let expected = ExpectedComparison {
        repository: PublicRepository {
            id: get("--repository-id")?
                .parse()
                .map_err(|_| "Invalid repository ID")?,
            owner: owner.into(),
            name: name.into(),
        },
        pr_number: get("--pr-number")?
            .parse()
            .map_err(|_| "Invalid PR number")?,
        selected_baseline: get("--selected-baseline")?.into(),
        baseline: get("--baseline")?.into(),
        proposal: get("--proposal")?.into(),
    };
    let policy: PublicPolicy =
        serde_json::from_value(read_json_file(Path::new(get("--policy")?), 64 * 1024)?)
            .map_err(|_| "Invalid strict public policy")?;
    let artifact = read_json_file(Path::new(get("--artifact")?), 16 * 1024 * 1024)?;
    let bundle = export_public_report(&artifact, &policy, &expected)?;
    write_bundle(&bundle, Path::new(get("--output")?))?;
    println!("{}", bundle.manifest.public_report_id);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Public export refused: {error}");
        std::process::exit(1);
    }
}
