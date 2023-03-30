use crate::util::MANAGER_NAME;
use k8s_openapi::{
    api::core::v1::{Container, EnvVar, Pod, VolumeMount},
    apimachinery::pkg::apis::meta::v1::Time,
};
use kube::{
    api::{Api, DeleteParams, Patch, PatchParams, PostParams, Resource},
    Client, CustomResourceExt,
};
use ytdl_common::{
    pod::{masked_pod, SHARED_PATH, SHARED_VOLUME_NAME},
    Error, DEFAULT_EXECUTOR_IMAGE,
};
use ytdl_types::{Executor, ExecutorPhase, ExecutorStatus};

/// Returns the image to use for the executor container.
/// It may be overridden by the user in the spec, but
/// defaults to the stock value in this project.
pub fn get_executor_image(instance: &Executor) -> String {
    instance
        .spec
        .executor
        .as_deref()
        .unwrap_or(DEFAULT_EXECUTOR_IMAGE)
        .to_owned()
}

/// A central tenet of this project is to only access
/// the external video service from within pods that
/// have VPN sidecars. Thus, both the video and the
/// thumbnail have to be downloaded by the proxy pod.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DownloadPodOptions {
    // If true, download the video to the storage backend.
    pub download_video: bool,

    // If true, download the thumbnail to the storage backend.
    pub download_thumbnail: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ProgressOptions {
    pub start_time: Option<Time>,
}

/// Returns the arguments to pass to the executor container's
/// default command. This is used to configure the executor
/// to download the video and/or thumbnail.
fn get_executor_args(options: DownloadPodOptions) -> Vec<String> {
    let mut args = vec!["download".to_owned()];
    if options.download_video {
        args.push("--download-video".to_owned());
    }
    if options.download_thumbnail {
        args.push("--download-thumbnail".to_owned());
    }
    args
}

/// Create the download pod for the given Executor.
/// The pod will have a VPN sidecar container, will
/// access the upload credentials from the cluster,
/// and will download the video and thumbnail to the
/// storage backend.
pub async fn create_pod(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
    service_account_name: String,
    options: DownloadPodOptions,
) -> Result<(), Error> {
    // Inject the spec as an environment variable.
    let resource: String = serde_json::to_string(instance)?;

    // Determine the executor image.
    let image = get_executor_image(instance);

    // Determine the executor args. The pod will use the
    // default command for the image and pass these as the
    // arguments.
    let args = get_executor_args(options);

    let container = Container {
        name: "executor".to_owned(),
        image: Some(image),
        // TODO: inject the imagePullPolicy from the helm chart.
        // There needs to be an ExecutorOptions struct corresponding to values.yaml->executor: (?)
        image_pull_policy: Some("Always".to_owned()), // FIXME: inject from helm
        args: Some(args),
        // Pass the full resource as an environment variable.
        env: Some(vec![EnvVar {
            name: "RESOURCE".to_owned(),
            value: Some(resource),
            ..EnvVar::default()
        }]),
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

    // Create the pod.
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    pod_api.create(&PostParams::default(), &pod).await?;
    Ok(())
}

/// Deletes the download pod for the given Executor.
pub async fn delete_pod(client: Client, name: &str, namespace: &str) -> Result<(), Error> {
    let api: Api<Pod> = Api::namespaced(client, namespace);
    api.delete(name, &DeleteParams::default()).await?;
    Ok(())
}

/// Marks the Executor's status as Succeeded.
pub async fn success(
    client: Client,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, instance, |status| {
        status.message = Some("download tasks completed without error".to_owned());
        status.phase = Some(ExecutorPhase::Succeeded);
    })
    .await?;
    Ok(())
}

/// Updates the Executor's status object to reflect download progress.
pub async fn progress(
    client: Client,
    instance: &Executor,
    start_time: Time,
) -> Result<(), Error> {
    patch_status(client, instance, |status| {
        status.message = Some("download tasks are in progress".to_owned());
        status.phase = Some(ExecutorPhase::Downloading);
        status.start_time = Some(start_time.0.to_rfc3339());
    })
    .await?;
    Ok(())
}

/// Updates the Executor's phase to Pending, which indicates
/// the resource made its initial appearance to the operator.
pub async fn pending(
    client: Client,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, instance, |status| {
        status.message = Some("the resource first appeared to the controller".to_owned());
        status.phase = Some(ExecutorPhase::Pending);
    })
    .await?;
    Ok(())
}

/// Update the Executor's phase to Starting, which indicates
/// the download pod is currently running.
pub async fn starting(
    client: Client,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, instance, |status| {
        status.message = Some("the download pod is starting".to_owned());
        status.phase = Some(ExecutorPhase::Starting);
    })
    .await?;
    Ok(())
}

pub async fn failure(
    client: Client,
    instance: &Executor,
    message: String,
) -> Result<(), Error> {
    patch_status(client, instance, move |status| {
        status.message = Some(message);
        status.phase = Some(ExecutorPhase::Failed);
    })
    .await?;
    Ok(())
}

/// Patch the Executor's status object with the provided function.
/// The function is passed a mutable reference to the status object,
/// which is to be mutated in-place. Move closures are supported.
async fn patch_status(
    client: Client,
    instance: &Executor,
    f: impl FnOnce(&mut ExecutorStatus),
) -> Result<Executor, Error> {
    let name = instance.metadata.name.as_deref().unwrap();
    let namespace = instance.metadata.namespace.as_deref().unwrap();
    let patch = Patch::Apply({
        let mut status = instance.status.clone().unwrap_or_default();
        f(&mut status);
        status.last_updated = Some(chrono::Utc::now().to_rfc3339());
        serde_json::json!({
            "apiVersion": "vpn.beebs.dev/v1",
            "kind": Executor::crd().spec.names.kind.clone(),
            "status": status,
        })
    });
    let api: Api<Executor> = Api::namespaced(client, namespace);
    Ok(api
        .patch_status(name, &PatchParams::apply(MANAGER_NAME), &patch)
        .await?)
}

pub mod finalizer {
    use super::*;
    use kube::api::{Patch, PatchParams};
    use serde_json::{json, Value};

    /// Adds a finalizer record into an `Executor` kind of resource. If the finalizer already exists,
    /// this action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Executor` resource with.
    /// - `name` - Name of the `Executor` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Executor` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn add(client: Client, name: &str, namespace: &str) -> Result<Executor, Error> {
        let api: Api<Executor> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": ["ytdl.beebs.dev/finalizer"]
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }

    /// Removes all finalizers from an `Executor` resource. If there are no finalizers already, this
    /// action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Executor` resource with.
    /// - `name` - Name of the `Executor` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Executor` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn delete(client: Client, name: &str, namespace: &str) -> Result<Executor, Error> {
        let api: Api<Executor> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": null
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }
}
