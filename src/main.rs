use std::sync::Arc;

use futures::stream::StreamExt;
use kube::Resource;
use kube::ResourceExt;
use kube::{
    api::ListParams,
    client::Client,
    runtime::controller::Action,
    runtime::Controller,
    Api,
};
use k8s_openapi::api::core::v1::{Pod, PodStatus};
use tokio::time::Duration;

use crate::crd::Video;

pub mod crd;
mod video;

use video::DownloadPodOptions;

#[tokio::main]
async fn main() {
    // First, a Kubernetes client must be obtained using the `kube` crate
    // The client will later be moved to the custom controller
    let kubernetes_client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");

    // Preparation of resources used by the `kube_runtime::Controller`
    let crd_api: Api<Video> = Api::all(kubernetes_client.clone());
    let context: Arc<ContextData> = Arc::new(ContextData::new(kubernetes_client.clone()));

    // The controller comes from the `kube_runtime` crate and manages the reconciliation process.
    // It requires the following information:
    // - `kube::Api<T>` this controller "owns". In this case, `T = Video`, as this controller owns the `Video` resource,
    // - `kube::api::ListParams` to select the `Video` resources with. Can be used for Video filtering `Video` resources before reconciliation,
    // - `reconcile` function with reconciliation logic to be called each time a resource of `Video` kind is created/updated/deleted,
    // - `on_error` function to call whenever reconciliation fails.
    Controller::new(crd_api.clone(), ListParams::default())
        .run(reconcile, on_error, context)
        .for_each(|reconciliation_result| async move {
            match reconciliation_result {
                Ok(video_resource) => {
                    println!("Reconciliation successful. Resource: {:?}", video_resource);
                }
                Err(reconciliation_err) => {
                    eprintln!("Reconciliation error: {:?}", reconciliation_err)
                }
            }
        })
        .await;
}

/// Context injected with each `reconcile` and `on_error` method invocation.
struct ContextData {
    /// Kubernetes client to make Kubernetes API requests with. Required for K8S resource management.
    client: Client,
}

impl ContextData {
    /// Constructs a new instance of ContextData.
    ///
    /// # Arguments:
    /// - `client`: A Kubernetes client to make Kubernetes REST API requests with. Resources
    /// will be created and deleted with this client.
    pub fn new(client: Client) -> Self {
        ContextData { client }
    }
}


enum VideoAction {
    // Create the download pod to download the video and/or thumbnail.
    CreateDownloadPod(DownloadPodOptions),

    // Delete the download pod.
    Delete,

    // We have the metadata but the database does not
    // have it yet and it needs it.
    WriteMetadata,
    
    // Nothing to do
    NoOp,
}

async fn reconcile(
    video: Arc<Video>,
    context: Arc<ContextData>,
) -> Result<Action, Error> {
    // The `Client` is shared -> a clone from the reference is obtained
    let client: Client = context.client.clone();
    
    let namespace: String = match video.namespace() {
        None => {
            // If there is no namespace to deploy to defined, reconciliation ends with an error immediately.
            return Err(Error::UserInputError(
                "Expected Video resource to be namespaced. Can't deploy to an unknown namespace."
                    .to_owned(),
            ));
        }
        // If namespace is known, proceed. In a more advanced version of the operator, perhaps
        // the namespace could be checked for existence first.
        Some(namespace) => namespace,
    };
    let name = video.name_any(); // Name of the Video resource is used to name the subresources as well.

    // Read phase of the reconciliation loop.
    let action = determine_action(client.clone(), &video).await?;
    
    if action != VideoAction::NoOp {
        // This log line is useful for debugging purposes.
        // Separate read & write phases greatly simplifies
        // the reconciliation loop. Deciding which actions
        // deserve their own enum entries may come down to
        // how badly you want to see them in the log, and
        // that alone is a perfectly valid reason to do so.
        println!("Action: {:?}", action);
    }

    // Write phase of the reconciliation loop.
    match action {
        VideoAction::CreateDownloadPod(options) => {
            // Apply the finalizer first. If that fails, the `?` operator invokes automatic conversion
            // of `kube::Error` to the `Error` defined in this crate.
            video::finalizer::add(client.clone(), &name, &namespace).await?;

            // Create the download pod.
            video::create_download_pod(client.clone(), &name, &namespace, options).await?;

            // Download pod will take at least one second to start.
            Ok(Action::requeue(Duration::from_secs(1)))
        }
        VideoAction::Delete => {
            // Deletes any subresources related to this `Video` resources. If and only if all subresources
            // are deleted, the finalizer is removed and Kubernetes is free to remove the `Video` resource.
            video::delete_download_pod(client.clone(), &name, &namespace).await?;

            // Once the deployment is successfully removed, remove the finalizer to make it possible
            // for Kubernetes to delete the `Video` resource (if needed)
            video::finalizer::delete(client, &name, &namespace).await?;

            // 
            Ok(Action::await_change())
        }
        VideoAction::WriteMetadata => Ok(Action::requeue(Duration::from_secs(10))),
        VideoAction::NoOp => {
            // The resource is already in desired state, do nothing and re-check after 10 seconds
            Ok(Action::requeue(Duration::from_secs(10)))
        },
    }
}

async fn needs_video_download(video: &Video) -> Result<bool, Error> {
    Ok(false)
}

async fn needs_thumbnail_download(video: &Video) -> Result<bool, Error> {
    Ok(false)
}

async fn get_download_pod(
    client: Client,
    video: &Video,
) -> Result<Option<Pod>, kube::Error> {
    let pod_api: Api<Pod> = Api::namespaced(client, &video.namespace().unwrap());
    let pod_name = video.name_any();
    match pod_api.get(&pod_name).await {
        Ok(pod) => Ok(Some(pod)),
        Err(e) => match &e {
            kube::Error::Api(ae) => match ae.code {
                // If the pod does not exist, return None
                404 => Ok(None),
                // If the pod exists but we can't access it, return an error
                _ => Err(e),
            },
            _ => Err(e),
        },
    }
}

async fn determine_download_action(
    client: Client,
    video: &Video,
    download_video: bool,
    download_thumbnail: bool,
) -> Result<VideoAction, Error> {
    // Check if the download pod already exists.
    let pod: Pod = match get_download_pod(client, video).await? {
        Some(pod) => pod,
        None => {
            // Create the download pod.
            return Ok(VideoAction::CreateDownloadPod(DownloadPodOptions {
                download_video,
                download_thumbnail,
            }));
        }
    };

    // Check the status of the download pod.
    let status: PodStatus = pod.status.ok_or(
        Error::UserInputError("Pod has no status".to_owned()))?;
    let phase: String = status.phase.ok_or(
        Error::UserInputError("Pod has no phase".to_owned()))?;
    match phase.as_str() {
        "Pending" => {
            // Download is not yet started.
            // TODO: report change to status object
            Ok(VideoAction::NoOp)
        }
        "Running" => {
            // Download is in progress.
            // TODO: report change to status object
            Ok(VideoAction::NoOp)
        }
        "Succeeded" => {
            // Download is completed. Clean up the pod
            // resource and move onto the next step.
            // TODO: report change to status object
            Ok(VideoAction::Delete)
        }
        _ => {
            // TODO: report error, delete pod, and re-create
            Ok(VideoAction::NoOp)
        }
    }
}

async fn determine_action(
    client: Client,
    video: &Video,
) -> Result<VideoAction, Error> {
    if video.meta().deletion_timestamp.is_some() {
        return Ok(VideoAction::Delete);
    };

    // TODO: determine if metadata needs to be written
    // to the database

    // Check if the video and/or thumbnail need to
    // be downloaded. Both of these operations must
    // occur inside a VPN-connected pod, so we will
    // do both tasks in the same pod.
    let result = tokio::join!(
        needs_video_download(video),
        needs_thumbnail_download(video),
    );
    let download_video = result.0?;
    let download_thumbnail = result.1?;
    if download_video || download_thumbnail {
        return determine_download_action(
            client,
            video,
            download_video,
            download_thumbnail,
        ).await;
    }
    
    Ok(VideoAction::NoOp)
}

/// Actions to be taken when a reconciliation fails - for whatever reason.
/// Prints out the error to `stderr` and requeues the resource for another reconciliation after
/// five seconds.
///
/// # Arguments
/// - `video`: The erroneous resource.
/// - `error`: A reference to the `kube::Error` that occurred during reconciliation.
/// - `_context`: Unused argument. Context Data "injected" automatically by kube-rs.
fn on_error(video: Arc<Video>, error: &Error, _context: Arc<ContextData>) -> Action {
    eprintln!("Reconciliation error:\n{:?}.\n{:?}", error, video);
    Action::requeue(Duration::from_secs(5))
}

/// All errors possible to occur during reconciliation
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any error originating from the `kube-rs` crate
    #[error("Kubernetes reported error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },
    /// Error in user input or Video resource definition, typically missing fields.
    #[error("Invalid Video CRD: {0}")]
    UserInputError(String),
}