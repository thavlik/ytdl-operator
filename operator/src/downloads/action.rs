use crate::util::MANAGER_NAME;
use k8s_openapi::api::core::v1::{Container, EnvVar, Pod, VolumeMount};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::{
    api::{Api, DeleteParams, Patch, PatchParams, PostParams, Resource},
    Client,
};
use ytdl_common::{
    pod::{masked_pod, SHARED_PATH, SHARED_VOLUME_NAME},
    Error, DEFAULT_EXECUTOR_IMAGE,
};
use ytdl_types::{Download, DownloadPhase, DownloadStatus};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ProgressOptions {
    pub start_time: Option<Time>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FailureOptions {
    pub message: String,
}

/// Deletes the query pod for the given Download.
pub async fn delete_query_pod(client: Client, name: &str, namespace: &str) -> Result<(), Error> {
    let api: Api<Pod> = Api::namespaced(client, namespace);
    api.delete(name, &DeleteParams::default()).await?;
    Ok(())
}

/// Returns the image to use for the executor container.
/// It may be overridden by the user in the spec, but
/// defaults to the stock value in this project.
pub fn get_executor_image(instance: &Download) -> String {
    instance
        .spec
        .executor
        .as_deref()
        .unwrap_or(DEFAULT_EXECUTOR_IMAGE)
        .to_owned()
}

/// Creates the query pod for the given Download.
pub async fn create_query_pod(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
    service_account_name: String,
) -> Result<(), Error> {
    // Determine the executor image.
    let image = get_executor_image(instance);

    let container = Container {
        name: "executor".to_owned(),
        image: Some(image),
        args: Some(vec!["query".to_owned()]),
        // TODO: inject the imagePullPolicy from the helm chart.
        // There needs to be an ExecutorOptions struct corresponding to values.yaml->executor: (?)
        image_pull_policy: Some("Always".to_owned()), // FIXME: inject from helm
        env: Some(vec![
            // Inject the spec as an environment variable.
            EnvVar {
                name: "RESOURCE".to_owned(),
                value: Some(serde_json::to_string(instance)?),
                ..EnvVar::default()
            },
        ]),
        // Pass the full resource as an environment variable.
        // We need the shared volume mounted as it contains
        // the unmasked IP retrieved during initialization.
        // The containers have a shared volume mounted at /share
        // that the VPN pod will write a file to when it's ready.
        // This way the executor pod can wait for the VPN to be
        // fully connected before starting any downloads.
        // Kubernetes does not provide robust enough means of
        // ensuring the VPN is connected before starting other
        // containers, so this is the best we can do.
        volume_mounts: Some(vec![VolumeMount {
            name: SHARED_VOLUME_NAME.to_owned(),
            mount_path: SHARED_PATH.to_owned(),
            ..VolumeMount::default()
        }]),
        ..Container::default()
    };

    // Make the Executor the owner of the pod.
    let oref = instance.controller_owner_ref(&()).unwrap();

    // Build the full Pod resource with the VPN sidecar.
    let pod: Pod = masked_pod(
        name.to_owned(),
        namespace.to_owned(),
        Some(vec![oref]),
        service_account_name,
        container,
    );
    let api: Api<Pod> = Api::namespaced(client, namespace);
    api.create(&PostParams::default(), &pod).await?;
    Ok(())
}

/// Updates the Download's status object to reflect download progress.
pub async fn download_progress(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
    succeeded: usize,
    total: usize,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some(format!(
            "download in progress ({}/{} succeeded)",
            succeeded, total
        ));
        status.phase = Some(DownloadPhase::Downloading.to_str().to_owned());
    })
    .await
}

/// Updates the Download's status object to signal complete success.
pub async fn succeeded(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("all downloads have succeeded".to_owned());
        status.phase = Some(DownloadPhase::Succeeded.to_str().to_owned());
    })
    .await
}

/// Updates the Download's status object to reflect query progress.
pub async fn query_progress(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
    start_time: Time,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("querying in progress".to_owned());
        status.phase = Some(DownloadPhase::Querying.to_str().to_owned());
        status.query_start_time = Some(start_time.0.to_rfc3339());
    })
    .await
}

/// Updates the Download's phase to Pending, which indicates
/// the resource made its initial appearance to the operator.
pub async fn pending(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("the resource first appeared to the controller".to_owned());
        status.phase = Some(DownloadPhase::Pending.to_str().to_owned());
    })
    .await
}

/// Update the Download's phase to Starting, which indicates
/// the query pod is initializing.
pub async fn query_starting(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("the query pod is starting".to_owned());
        status.phase = Some(DownloadPhase::QueryStarting.to_str().to_owned());
    })
    .await
}

/// Updates the Download's status object to reflect query failure.
pub async fn query_failure(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
    message: String,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, move |status| {
        status.message = Some(message);
        status.phase = Some(DownloadPhase::ErrQueryFailed.to_str().to_owned());
    })
    .await
}

/// Patch the Download's status object with the provided function.
/// The function is passed a mutable reference to the status object,
/// which is to be mutated in-place. Move closures are supported.
async fn patch_status(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Download,
    f: impl FnOnce(&mut DownloadStatus),
) -> Result<(), Error> {
    let patch = Patch::Apply({
        let mut instance: Download = instance.clone();
        let status: &mut DownloadStatus = match instance.status.as_mut() {
            Some(status) => status,
            None => {
                // Create the status object.
                instance.status = Some(DownloadStatus::default());
                instance.status.as_mut().unwrap()
            }
        };
        f(status);
        let now = chrono::Utc::now().to_rfc3339();
        status.last_updated = Some(now);
        instance
    });
    let api: Api<Download> = Api::namespaced(client, namespace);
    api.patch(name, &PatchParams::apply(MANAGER_NAME), &patch)
        .await?;
    Ok(())
}

pub mod finalizer {
    use super::*;
    use kube::api::{Patch, PatchParams};
    use serde_json::{json, Value};

    /// Adds a finalizer record into an `Download` kind of resource. If the finalizer already exists,
    /// this action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Download` resource with.
    /// - `name` - Name of the `Download` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Download` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn add(client: Client, name: &str, namespace: &str) -> Result<Download, Error> {
        let api: Api<Download> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": ["ytdl.beebs.dev/finalizer"]
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }

    /// Removes all finalizers from an `Download` resource. If there are no finalizers already, this
    /// action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Download` resource with.
    /// - `name` - Name of the `Download` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Download` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn delete(client: Client, name: &str, namespace: &str) -> Result<Download, Error> {
        let api: Api<Download> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": null
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }
}
