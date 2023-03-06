use k8s_openapi::api::core::v1::{
    Capabilities, Container, EmptyDirVolumeSource, EnvVar, EnvVarSource, Pod, PodSpec,
    SecretKeySelector, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::{DeleteParams, ObjectMeta, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use std::collections::BTreeMap;
use ytdl_common::{Error, IP_FILE_PATH, IP_SERVICE};
use ytdl_types::{Executor, ExecutorPhase, ExecutorStatus};

/// Friendly name for the controller.
const MANAGER_NAME: &str = "ytdl-operator";

/// Default image to use for the executor. The executor
/// image is responsible for downloading the video and
/// thumbnail from the video service, and uploading them
/// to the storage backend in the desired formats.
const DEFAULT_EXECUTOR_IMAGE: &str = "thavlik/ytdl-executor:latest";

/// VPN sidecar image. Efforts were made to use a stock
/// image with no modifications, as to maximize the
/// plug-and-play nature of the image.
const DEFAULT_VPN_IMAGE: &str = "qmcgaw/gluetun:v3.32.0";

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FailureOptions {
    pub message: String,
}

/// Creates the container spec for the VPN sidecar.
pub fn get_vpn_sidecar() -> Container {
    Container {
        name: "vpn".to_owned(),
        image: Some(DEFAULT_VPN_IMAGE.to_owned()),
        security_context: Some(SecurityContext {
            capabilities: Some(Capabilities {
                add: Some(vec!["NET_ADMIN".to_owned()]),
                ..Capabilities::default()
            }),
            ..SecurityContext::default()
        }),
        env: Some(vec![
            // TODO: configure gluetun env vars
            // https://github.com/qdm12/gluetun/wiki/
            EnvVar {
                name: "VPN_SERVICE_PROVIDER".to_owned(),
                value: Some("private internet access".to_owned()),
                ..EnvVar::default()
            },
            EnvVar {
                name: "IP_SERVICE".to_owned(),
                value: Some(IP_SERVICE.to_owned()),
                ..EnvVar::default()
            },
            EnvVar {
                name: "OPENVPN_USER".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("pia-creds".to_owned()),
                        key: "username".to_owned(),
                        ..SecretKeySelector::default()
                    }),
                    ..EnvVarSource::default()
                }),
                ..EnvVar::default()
            },
            EnvVar {
                name: "OPENVPN_PASSWORD".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("pia-creds".to_owned()),
                        key: "password".to_owned(),
                        ..SecretKeySelector::default()
                    }),
                    ..EnvVarSource::default()
                }),
                ..EnvVar::default()
            },
        ]),
        ..Container::default()
    }
}

/// Creates the container spec for the init container that
/// retrieves the unmasked public IP address and writes it
/// to the shared volume. This is done on startup so that
/// the executor will truly know when it's okay to start
/// downloading the video and/or thumbnail.
fn get_init_container() -> Container {
    Container {
        name: "init".to_owned(),
        image: Some("curlimages/curl:7.88.1".to_owned()),
        image_pull_policy: Some("IfNotPresent".to_owned()),
        command: Some(
            vec!["curl", "-o", IP_FILE_PATH, "-s", IP_SERVICE]
                .into_iter()
                .map(|s| s.to_owned())
                .collect(),
        ),
        volume_mounts: Some(vec![VolumeMount {
            name: "shared".to_owned(),
            mount_path: "/shared".to_owned(),
            ..VolumeMount::default()
        }]),
        ..Container::default()
    }
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
) -> Result<Pod, Error> {
    // Inject the spec as an environment variable.
    let resource: String = serde_json::to_string(instance)?;

    // Determine the executor image.
    let image: String = instance
        .spec
        .executor
        .as_deref()
        .unwrap_or(DEFAULT_EXECUTOR_IMAGE)
        .to_owned();

    // Run command varies based on what needs downloaded.
    let mut command = vec!["ytdl-executor".to_owned()];
    if options.download_video {
        command.push("--download-video".to_owned());
    }
    if options.download_thumbnail {
        command.push("--download-thumbnail".to_owned());
    }

    // Each executor will have a VPN sidecar to avoid
    // drawing attention from the video service.
    let vpn_sidecar = get_vpn_sidecar();

    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert("app".to_owned(), "ytdl".to_owned());

    // The containers have a shared volume mounted at /share
    // that the VPN pod will write a file to when it's ready.
    // This way the executor pod can wait for the VPN to be
    // fully connected before starting any downloads.
    // Kubernetes does not provide robust enough means of
    // ensuring the VPN is connected before starting other
    // containers, so this is the best we can do.
    let pod: Pod = Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            labels: Some(labels),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            restart_policy: Some("Never".to_owned()),
            service_account_name: Some(service_account_name),
            // Create an init container that writes the unmasked public
            // IP to a shared file. This container must complete before
            // the others can start, and this is useful when the executor
            // is trying to figure out the moment the VPN is connected.
            init_containers: Some(vec![get_init_container()]),
            containers: vec![
                // Kubelet will start the VPN sidecar first.
                vpn_sidecar,
                // Starting the executor container last may reduce VPN
                // connection wait time.
                Container {
                    name: "executor".to_owned(),
                    image: Some(image),
                    command: Some(command),
                    env: Some(vec![EnvVar {
                        name: "RESOURCE".to_owned(),
                        value: Some(resource),
                        ..EnvVar::default()
                    }]),
                    volume_mounts: Some(vec![VolumeMount {
                        name: "shared".to_owned(),
                        mount_path: "/shared".to_owned(),
                        ..VolumeMount::default()
                    }]),
                    ..Container::default()
                },
            ],
            volumes: Some(vec![Volume {
                name: "shared".to_owned(),
                empty_dir: Some(EmptyDirVolumeSource {
                    ..EmptyDirVolumeSource::default()
                }),
                ..Volume::default()
            }]),
            ..PodSpec::default()
        }),
        ..Pod::default()
    };

    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    Ok(pod_api.create(&PostParams::default(), &pod).await?)
}

pub async fn delete_pod(client: Client, name: &str, namespace: &str) -> Result<(), Error> {
    let api: Api<Pod> = Api::namespaced(client, namespace);
    api.delete(name, &DeleteParams::default()).await?;
    Ok(())
}

/// Marks the Executor's status as Succeeded.
pub async fn success(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("download tasks completed without error".to_owned());
        status.phase = Some(ExecutorPhase::Succeeded.to_str().to_owned());
    })
    .await
}

/// Updates the Executor's status object to reflect download progress.
pub async fn progress(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
    start_time: Time,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("download tasks are in progress".to_owned());
        status.phase = Some(ExecutorPhase::Downloading.to_str().to_owned());
        status.start_time = Some(start_time.0.to_rfc3339());
    })
    .await
}

/// Updates the Executor's phase to Pending, which indicates
/// the resource made its initial appearance to the operator.
pub async fn pending(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("the resource first appeared to the controller".to_owned());
        status.phase = Some(ExecutorPhase::Pending.to_str().to_owned());
    })
    .await
}

/// Update the Executor's phase to Starting, which indicates
/// the download pod is currently running.
pub async fn starting(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, |status| {
        status.message = Some("the download pod is starting".to_owned());
        status.phase = Some(ExecutorPhase::Starting.to_str().to_owned());
    })
    .await
}

pub async fn failure(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
    options: FailureOptions,
) -> Result<(), Error> {
    patch_status(client, name, namespace, instance, move |status| {
        status.message = Some(options.message);
        status.phase = Some(ExecutorPhase::Failed.to_str().to_owned());
    })
    .await
}

/// Patch the Executor's status object with the provided function.
/// The function is passed a mutable reference to the status object,
/// which is to be mutated in-place. Move closures are supported.
async fn patch_status(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
    f: impl FnOnce(&mut ExecutorStatus),
) -> Result<(), Error> {
    let patch = Patch::Apply({
        let mut instance: Executor = instance.clone();
        let status: &mut ExecutorStatus = match instance.status.as_mut() {
            Some(status) => status,
            None => {
                // Create the status object.
                instance.status = Some(ExecutorStatus::default());
                instance.status.as_mut().unwrap()
            }
        };
        f(status);
        let now = chrono::Utc::now().to_rfc3339();
        status.last_updated = Some(now);
        instance
    });
    let api: Api<Executor> = Api::namespaced(client, namespace);
    api.patch(name, &PatchParams::apply(MANAGER_NAME), &patch)
        .await?;
    Ok(())
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
