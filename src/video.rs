use crate::crd::{Video, VideoPhase, VideoStatus};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::{DeleteParams, ObjectMeta, Patch, PatchParams, PostParams};
use kube::ResourceExt;
use kube::{Api, Client, Error};
use std::collections::BTreeMap;

const MANAGER_NAME: &str = "ytdl-operator";

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
    options: DownloadPodOptions,
) -> Result<Pod, Error> {
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert("app".to_owned(), name.to_owned());

    // TODO: configure vpn sidecar

    let pod: Pod = Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            labels: Some(labels.clone()),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            containers: vec![
                Container {
                    name: "executor".to_owned(),
                    image: Some("thavlik/ytdl-executor:latest".to_owned()),
                    ..Container::default()
                },
                Container {
                    name: "nordvpn".to_owned(),
                    image: Some("thavlik/nordvpn:latest".to_owned()),
                    ..Container::default()
                },
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

/// Marks the Video's status as Succeeded.
pub async fn success(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
) -> Result<(), Error> {
    patch_status(client, name, namespace, video, |status| {
        status.message = Some("download tasks completed without error".to_owned());
        status.phase = Some(VideoPhase::Succeeded.to_str().to_owned());
    })
    .await
}

/// Updates the Video's status object to reflect download progress.
pub async fn progress(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
    start_time: Time,
) -> Result<(), Error> {
    patch_status(client, name, namespace, video, |status| {
        status.message = Some("download tasks are in progress".to_owned());
        status.phase = Some(VideoPhase::Downloading.to_str().to_owned());
        status.start_time = Some("TODO: format time".to_owned());
    })
    .await
}

/// Updates the Video's phase to Pending, which indicates
/// the resource made its initial appearance to the operator.
pub async fn pending(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
) -> Result<(), Error> {
    patch_status(client, name, namespace, video, |status| {
        status.message = Some("the resource first appeared to the controller".to_owned());
        status.phase = Some(VideoPhase::Pending.to_str().to_owned());
    })
    .await
}

/// Update the Video's phase to Starting, which indicates
/// the download pod is currently running.
pub async fn starting(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
) -> Result<(), Error> {
    patch_status(client, name, namespace, video, |status| {
        status.message = Some("the download pod is starting".to_owned());
        status.phase = Some(VideoPhase::Starting.to_str().to_owned());
    })
    .await
}

pub async fn failure(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
    options: FailureOptions,
) -> Result<(), Error> {
    patch_status(client, name, namespace, video, move |status| {
        status.message = Some(options.message);
        status.phase = Some(VideoPhase::Failed.to_str().to_owned());
    })
    .await
}

/// Patch the Video's status object with the provided function.
/// The function is passed a mutable reference to the status object,
/// which is to be mutated in-place. Move closures are supported.
async fn patch_status(
    client: Client,
    name: &str,
    namespace: &str,
    video: &Video,
    f: impl FnOnce(&mut VideoStatus),
) -> Result<(), Error> {
    let patch = Patch::Apply({
        let mut video: Video = video.clone();
        let status: &mut VideoStatus = match video.status.as_mut() {
            Some(status) => status,
            None => {
                // Create the status object.
                video.status = Some(VideoStatus::default());
                video.status.as_mut().unwrap()
            }
        };
        f(status);
        status.last_updated = Some("TODO: now".to_owned()); // TODO: figure out timestamps
        video
    });
    let video_api: Api<Video> = Api::namespaced(client, namespace);
    video_api
        .patch(name, &PatchParams::apply(MANAGER_NAME), &patch)
        .await?;
    Ok(())
}

pub mod finalizer {
    use super::*;
    use kube::api::{Patch, PatchParams};
    use serde_json::{json, Value};

    /// Adds a finalizer record into an `Video` kind of resource. If the finalizer already exists,
    /// this action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Video` resource with.
    /// - `name` - Name of the `Video` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Video` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn add(client: Client, name: &str, namespace: &str) -> Result<Video, Error> {
        let api: Api<Video> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": ["ytdl.org/finalizer"]
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }

    /// Removes all finalizers from an `Video` resource. If there are no finalizers already, this
    /// action has no effect.
    ///
    /// # Arguments:
    /// - `client` - Kubernetes client to modify the `Video` resource with.
    /// - `name` - Name of the `Video` resource to modify. Existence is not verified
    /// - `namespace` - Namespace where the `Video` resource with given `name` resides.
    ///
    /// Note: Does not check for resource's existence for simplicity.
    pub async fn delete(client: Client, name: &str, namespace: &str) -> Result<Video, Error> {
        let api: Api<Video> = Api::namespaced(client, namespace);
        let finalizer: Value = json!({
            "metadata": {
                "finalizers": null
            }
        });
        let patch: Patch<&Value> = Patch::Merge(&finalizer);
        Ok(api.patch(name, &PatchParams::default(), &patch).await?)
    }
}
