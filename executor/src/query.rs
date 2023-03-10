use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    api::{ObjectMeta, PostParams},
    client::Client,
    Api, ResourceExt,
};
use std::{collections::BTreeMap, env, process::Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use ytdl_common::{create_executor, get_executor, Error, INFO_JSONL_KEY};
use ytdl_types::Download;

fn build_args(url: &str, ignore_errors: bool) -> Vec<&str> {
    let mut args = vec!["-j"];
    if ignore_errors {
        args.push("--ignore-errors");
    }
    args.push(url);
    args
}

/*
/// Queries the video metadata from the given url.
pub async fn simple_query(command: &str, url: &str, ignore_errors: bool) -> Result<Vec<String>, Error> {
    let mut child = Command::new(command)
        .args(&build_args(url, ignore_errors)[..])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::UnknownError("failed to get child process stdout".to_owned()))?;
    // Read the output line-by-line.
    let mut reader = BufReader::new(stdout).lines();
    let mut lines = Vec::new();
    while let Some(line) = reader.next_line().await? {
        // TODO: reconcile the Executor for this line.
        println!("{}", line);
        lines.push(line);
    }
    // Wait for the command to exit.
    let status = child.wait().await?;
    if status.success() {
        return Ok(lines);
    }
    Err(Error::UnknownError(format!(
        "youtube-dl exited with status code {}",
        status.code().unwrap_or(-1)
    )))
}
*/

/// Try to reconcile the Executor associated with this json metadata.
async fn reconcile_executor(
    client: Client,
    instance: &Download,
    id: &str,
    line: &str,
) -> Result<(), Error> {
    if get_executor(
        client.clone(),
        &format!("{}-{}", instance.name_any(), id),
        instance.namespace().as_ref().unwrap(),
    )
    .await?
    .is_none()
    {
        // Create the Executor from this line of output.
        println!("Creating Executor for {}", id);
        create_executor(client, instance, id.to_owned(), line.to_owned()).await?;
    }
    Ok(())
}

/// Parses the Download resource from the environment.
fn get_resource() -> Result<Download, Error> {
    Ok(serde_json::from_str(&env::var("RESOURCE")?)?)
}

/// Queries the video metadata from the given url and creates
/// Executor resources as needed.
pub async fn query(client: Client, command: &str) -> Result<(), Error> {
    let instance: Download = get_resource()?;

    // Wait for the VPN to connect before starting the query.
    println!("Environment parsed, waiting for VPN to connect");
    crate::ready::wait_for_vpn().await?;

    // Build the args for the youtube-dl command.
    let args = build_args(
        &instance.spec.query,
        instance.spec.ignore_errors.unwrap_or(false),
    );

    // Start the youtube-dl command.
    let mut child = Command::new(command)
        .args(&args[..])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::UnknownError("failed to get child process stdout".to_owned()))?;

    // Read the output line-by-line.
    let mut reader = BufReader::new(stdout).lines();
    let mut lines = Vec::new();
    while let Some(line) = reader.next_line().await? {
        // Immediately dump the line to the console.
        println!("{}", line);

        // Try and parse the line as json.
        let info_json: serde_json::Value = match serde_json::from_str(&line) {
            Ok(info_json) => info_json,
            Err(err) => {
                // Ignore this line.
                println!("Failed to parse json: {}", err);
                continue;
            }
        };

        // All youtube-dl info json should have an "id" field.
        let id: &str = match info_json["id"].as_str() {
            Some(id) => id,
            None => {
                // Ignore this line.
                println!("Failed to parse id from json");
                continue;
            }
        };

        // Try and create an Executor for the video.
        if let Err(err) = reconcile_executor(client.clone(), &instance, id, &line).await {
            println!("Failed to create Executor for {}: {}", id, err);
        }

        // Add the line to the final output ConfigMap, as we know it's valid json.
        lines.push(line);
    }

    // Wait for the command to exit.
    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::UnknownError(format!(
            "youtube-dl exited with status code {}",
            status.code().unwrap_or(-1)
        )));
    }

    // Upload the metadata as a ConfigMap.
    println!("Creating metadata ConfigMap ({} lines)", lines.len());
    publish_metadata(client, &instance, lines).await?;

    // All done.
    println!("Successfully queried metadata for {}", &instance.spec.query);
    Ok(())
}

async fn publish_metadata(
    client: Client,
    instance: &Download,
    lines: Vec<String>,
) -> Result<(), Error> {
    let namespace = instance.namespace().unwrap();
    let api: Api<ConfigMap> = Api::namespaced(client, &namespace);
    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(instance.name_any()),
            namespace: Some(namespace),
            ..Default::default()
        },
        data: Some({
            let mut data = BTreeMap::new();
            data.insert(INFO_JSONL_KEY.to_owned(), lines.join("\n"));
            data
        }),
        ..Default::default()
    };
    api.create(&PostParams::default(), &cm).await?;
    Ok(())
}
