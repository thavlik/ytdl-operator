use k8s_openapi::api::core::v1::{
    Container, EnvVar, EnvVarSource, Pod, PodSpec, SecretKeySelector,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::{DeleteParams, ObjectMeta, Patch, PatchParams, PostParams};
use kube::ResourceExt;
use kube::{Api, Client, Error};
use std::collections::BTreeMap;
use ytdl_operator_types::{Executor, ExecutorPhase, ExecutorStatus};

const MANAGER_NAME: &str = "ytdl-operator";
const DEFAULT_EXECUTOR_IMAGE: &str = "thavlik/ytdl-executor:latest";

/// A central tenet of this project is to only access
/// the external video service from within pods that
/// have VPN sidecars. Thus, both the video and the
/// thumbnail have to be downloaded by the proxy pod.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DownloadPodOptions {
    // if true, download the video to the storage backend
    pub download_video: bool,

    // if true, download the thumbnail to the storage backend
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

pub async fn create_download_pod(
    client: Client,
    name: &str,
    namespace: &str,
    instance: &Executor,
    service_account_name: String,
    options: DownloadPodOptions,
) -> Result<Pod, Error> {
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert("app".to_owned(), name.to_owned());

    // Inject the spec as an environment variable.
    // Properly handling the error here is nontrivial
    // because this function returns kube errors only.
    // In any case, this should never fail, and if it
    // does, it's a serious bug that warrants detecting.
    let spec: String =
        serde_json::to_string(&instance.spec).expect("failed to marshal spec to json");

    // Determine the executor image.
    let image: String = instance
        .spec
        .executor
        .as_deref()
        .unwrap_or(DEFAULT_EXECUTOR_IMAGE)
        .to_owned();

    // VPN sidecar container. I've personally tested
    // NordVPN on k8s, but it should work with any.
    // TODO: support more VPNs
    // TODO: https://github.com/thavlik/vpn-operator
    let vpn_sidecar = Container {
        name: "nordvpn".to_owned(),
        image: Some("thavlik/nordvpn:latest".to_owned()),
        env: Some(vec![
            EnvVar {
                name: "NORD_USERNAME".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("nordvpn-creds".to_owned()),
                        key: "username".to_owned(),
                        ..SecretKeySelector::default()
                    }),
                    ..EnvVarSource::default()
                }),
                ..EnvVar::default()
            },
            EnvVar {
                name: "NORD_PASSWORD".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("nordvpn-creds".to_owned()),
                        key: "password".to_owned(),
                        ..SecretKeySelector::default()
                    }),
                    ..EnvVarSource::default()
                }),
                ..EnvVar::default()
            },
        ]),
        ..Container::default()
    };

    let pod: Pod = Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            labels: Some(labels),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            service_account_name: Some(service_account_name),
            containers: vec![
                Container {
                    name: "executor".to_owned(),
                    image: Some(image),
                    env: Some(vec![
                        EnvVar {
                            name: "SPEC".to_owned(),
                            value: Some(spec),
                            ..EnvVar::default()
                        },
                        EnvVar {
                            name: "NAMESPACE".to_owned(),
                            value: Some(namespace.to_owned()),
                            ..EnvVar::default()
                        },
                    ]),
                    ..Container::default()
                },
                vpn_sidecar,
            ],
            ..PodSpec::default()
        }),
        ..Pod::default()
    };

    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    pod_api.create(&PostParams::default(), &pod).await
}

pub async fn delete_download_pod(client: Client, name: &str, namespace: &str) -> Result<(), Error> {
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
        status.start_time = Some("TODO: format time".to_owned());
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
        status.last_updated = Some("TODO: now".to_owned()); // TODO: figure out timestamps
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
                "finalizers": ["ytdl.org/finalizer"]
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
