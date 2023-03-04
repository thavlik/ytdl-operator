use std::sync::Arc;

use futures::stream::StreamExt;
use k8s_openapi::api::core::v1::{Pod, PodStatus};
use k8s_openapi::chrono::Utc;
use kube::Resource;
use kube::ResourceExt;
use kube::{
    api::ListParams, client::Client, runtime::controller::Action, runtime::Controller, Api,
};
use tokio::time::Duration;

use awsregion::Region;
use s3::bucket::Bucket;
use s3::creds::Credentials;

use crate::crd::{Video, VideoPhase};

pub mod crd;
mod video;

use video::{DownloadPodOptions, FailureOptions, ProgressOptions};

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

#[derive(Debug, PartialEq, Eq, Clone)]
enum VideoAction {
    // The resource first appeared to the controller and requires
    // its phase to be set to "Pending" to indicate that reconciliation
    // is in progress.
    SetPending,

    // Create the pod to download the video and/or thumbnail. Subsequent
    // reconciliations will update the Video's status to reflect the
    // progress of the download.
    CreateDownloadPod(DownloadPodOptions),

    // Delete the download pod. This is done when the Video resource is
    // deleted and when the download pod needs to be deleted to proceed
    // with reconciliation.
    Delete,

    // The download pod is still downloading the video and/or thumbnail.
    SetProgress(ProgressOptions),

    // Download pod has finished downloading the video and/or thumbnail.
    SetSucceeded,

    // Download pod has failed with an error message.
    SetFailure(FailureOptions),

    // Nothing to do (reconciliation successful)
    NoOp,
}

/// Main reconciliation loop for the `Video` resource.
async fn reconcile(video: Arc<Video>, context: Arc<ContextData>) -> Result<Action, Error> {
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
        println!("{}/{} ACTION: {:?}", namespace, name, action);
    }

    // Write phase of the reconciliation loop.
    match action {
        VideoAction::SetPending => {
            // Update the status of the resource to reflect that reconciliation is in progress.
            video::pending(client, &name, &namespace, video.as_ref()).await?;

            // Requeue the resource to be immediately reconciled again.
            Ok(Action::requeue(Duration::ZERO))
        }
        VideoAction::CreateDownloadPod(options) => {
            // Apply the finalizer first. This way the Video resource
            // won't be deleted before the download pod is deleted.
            video::finalizer::add(client.clone(), &name, &namespace).await?;

            // Create the download pod.
            video::create_download_pod(client.clone(), &name, &namespace, options).await?;

            // Update the phase to reflect that the download has started.
            video::starting(client, &name, &namespace, video.as_ref()).await?;

            // Download pod will take at least a couple seconds to start.
            Ok(Action::requeue(Duration::from_secs(3)))
        }
        VideoAction::SetProgress(options) => {
            // Update the status of the resource to reflect that the
            // download is in progress. In the case that no start time
            // is provided, set the Video phase to "Starting".
            match options.start_time {
                // Post progress event with start time.
                Some(start_time) => {
                    video::progress(
                        client.clone(),
                        &name,
                        &namespace,
                        video.as_ref(),
                        start_time,
                    )
                    .await?
                }
                // Indicate that the downloads are starting.
                None => video::starting(client.clone(), &name, &namespace, video.as_ref()).await?,
            }

            // Requeue the resource to be reconciled again. Expect
            // the download(s) to take at least a couple seconds
            // before completion occurs.
            Ok(Action::requeue(Duration::from_secs(3)))
        }
        VideoAction::SetSucceeded => {
            // Update the status of the resource to reflect download completion.
            video::success(client, &name, &namespace, video.as_ref()).await?;

            // Requeue immediately.
            Ok(Action::requeue(Duration::ZERO))
        }
        VideoAction::SetFailure(options) => {
            // Update the status of the resource to communicate the error.
            video::failure(client.clone(), &name, &namespace, video.as_ref(), options).await?;

            // Delete the download pod so it can be recreated.
            video::delete_download_pod(client, &name, &namespace).await?;

            // Requeue immediately.
            Ok(Action::requeue(Duration::ZERO))
        }
        VideoAction::Delete => {
            // Deletes any subresources related to this `Video` resources. If and only if all subresources
            // are deleted, the finalizer is removed and Kubernetes is free to remove the `Video` resource.
            video::delete_download_pod(client.clone(), &name, &namespace).await?;

            // Once the deployment is successfully removed, remove the finalizer to make it possible
            // for Kubernetes to delete the `Video` resource (if needed)
            video::finalizer::delete(client, &name, &namespace).await?;

            if video.meta().deletion_timestamp.is_some() {
                // No need to requeue deleted objects.
                return Ok(Action::await_change());
            }

            // Delete was requested explicitly and the resource isn't pending deletion.
            // Requeue the resource to be immediately reconciled again.
            Ok(Action::requeue(Duration::ZERO))
        }
        VideoAction::NoOp => {
            // Nothing to do (resource is fully reconciled).
            Ok(Action::await_change())
        }
    }
}

/// Returns true if the bucket has an object with the given key
/// and the object is not empty (i.e. corrupt or incomplete).
async fn bucket_has_obj(bucket: Bucket, key: &str) -> Result<bool, Error> {
    let (head, code) = bucket.head_object(key).await?;
    if code == 404 {
        // The object does not exist
        return Ok(true);
    }
    Ok(head.content_length.unwrap_or(0) > 0)
}

/// Returns true if the video needs to be downloaded.
async fn needs_video_download(client: Client, video: &Video) -> Result<bool, Error> {
    let bucket = get_video_bucket(client, video).await?;
    // TODO: extract video key from resource
    let key = video.name_any();
    bucket_has_obj(bucket, &key).await
}

/// Returns true if the thumbnail needs to be downloaded.
async fn needs_thumbnail_download(client: Client, video: &Video) -> Result<bool, Error> {
    let bucket = get_thumbnail_bucket(client, video).await?;
    let key = video.name_any();
    bucket_has_obj(bucket, &key).await
}

/// Returns the download pod if it exists, or None if it does not.
async fn get_download_pod(client: Client, video: &Video) -> Result<Option<Pod>, kube::Error> {
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

/// Returns a tuple of booleans indicating whether the video
/// and/or the thumbnail should be downloaded. Both checks
/// are made concurrently for maximum performance.
async fn check_downloads(client: Client, video: &Video) -> Result<(bool, bool), Error> {
    // TODO: only check for thumbnail if configured to do so
    // TODO: only check for video if configured to do so
    let result = tokio::join!(
        needs_video_download(client.clone(), video),
        needs_thumbnail_download(client, video),
    );
    let download_video = result.0?;
    let download_thumbnail = result.1?;
    Ok((download_video, download_thumbnail))
}

/// Returns the Bucket to be used for video file storage.
async fn get_video_bucket(client: Client, video: &Video) -> Result<Bucket, Error> {
    // TODO: properly extract bucket name from resource
    let bucket_name = "rust-s3-test";
    let region_name = "nyc3".to_string();
    let endpoint = "https://nyc3.digitaloceanspaces.com".to_string();
    let region = Region::Custom {
        region: region_name,
        endpoint,
    };
    // TODO: get s3 credentials from kubernetes secret
    let credentials = Credentials::default()?;
    Ok(Bucket::new(bucket_name, region, credentials)?)
}

/// Returns the Bucket to be used for thumbnail storage.
async fn get_thumbnail_bucket(client: Client, video: &Video) -> Result<Bucket, Error> {
    // TODO: same code but for thumbnail bucket
    get_video_bucket(client, video).await
}

/// Returns the phase of the video.
fn get_phase(video: &Video) -> Result<VideoPhase, Error> {
    let phase: &str = video.status.as_ref().unwrap().phase.as_ref().unwrap();
    let phase: VideoPhase =
        VideoPhase::from_str(phase).ok_or_else(|| Error::InvalidPhase(phase.to_string()))?;
    Ok(phase)
}

/// Determines the action to take after all downloads have completed.
/// The controller will first set the Video phase to Succeeded, then
/// it will delete the download pod.
async fn determine_download_success_action(
    client: Client,
    video: &Video,
) -> Result<Option<VideoAction>, Error> {
    if get_phase(video)? != VideoPhase::Succeeded {
        // Mark the Video resource as succeeded before
        // garbage collecting the download pod.
        return Ok(Some(VideoAction::SetSucceeded));
    }
    match get_download_pod(client, video).await? {
        // Garbage collect the download pod. Given that
        // the Delete action is invoked after the pod
        // succeeds, this branch *shouldn't* be reached,
        // but for safety we handle it anyway.
        Some(_) => Ok(Some(VideoAction::Delete)),
        // Do nothing and proceed with reconciliation.
        None => Ok(None),
    }
}

/// Determines the action to take given that the download pod
/// exists and we need to check its status.
async fn determine_download_pod_action(
    client: Client,
    video: &Video,
    pod: Pod,
) -> Result<Option<VideoAction>, Error> {
    // Check the status of the download pod.
    let status: &PodStatus = pod
        .status
        .as_ref()
        .ok_or_else(|| Error::GenericError("download pod has no status".to_owned()))?;
    let phase: &str = status
        .phase
        .as_ref()
        .ok_or_else(|| Error::GenericError("download pod has no phase".to_owned()))?;
    match phase {
        "Pending" => {
            // Download is not yet started.
            if status.conditions.is_some() {
                // Check for scheduling problems.
                let conditions: &Vec<_> = status.conditions.as_ref().unwrap();
                for condition in conditions {
                    if condition.type_ == "PodScheduled" && condition.status == "False" {
                        let message = format!(
                            "download pod is not scheduled: {}",
                            condition.message.as_ref().unwrap()
                        );
                        return Ok(Some(VideoAction::SetFailure(FailureOptions { message })));
                    }
                }
            }
            // Download pod is Pending without error.
            // Mark the Video phase as being in-progress.
            Ok(Some(VideoAction::SetProgress(ProgressOptions {
                start_time: None,
            })))
        }
        "Running" => {
            // Download is in progress.
            // TODO: report verbose download statistics.
            Ok(Some(VideoAction::SetProgress(ProgressOptions {
                start_time: pod.creation_timestamp(),
            })))
        }
        "Succeeded" => {
            // Download is completed. Delete the pod. When the
            // Video is requeued, the files will not need downloading,
            // and the pod will be deleted after the Video phase is
            // flagged as `Succeeded`.
            Ok(Some(VideoAction::Delete))
        }
        _ => {
            // Report error, delete pod, and re-create.
            // TODO: find way to extract a verbose error message from the pod.
            let message = format!("pod is in phase {}", phase);
            Ok(Some(VideoAction::SetFailure(FailureOptions { message })))
        }
    }
}

/// Determines the action to take for a Video resource concerning
/// the files that need to be downloaded. If no files need to be
/// downloaded, the returned action is None, signifying that
/// reconciliation should proceed to the next phase.
async fn determine_download_action(
    client: Client,
    video: &Video,
) -> Result<Option<VideoAction>, Error> {
    // We don't want to HEAD the bucket on every loop, so this
    // is optimized by checking the status of the download pod
    // first, as its existence implies that there were files
    // that previously needed downloading.
    let pod: Pod = match get_download_pod(client.clone(), video).await? {
        // Download pod exists, no reason to check storage
        // as the results of `check_downloads` are cached
        // in the pod's spec.
        Some(pod) => pod,
        // Download pod does not exist, check storage to see
        // which files, if any, require downloading.
        None => {
            // Determine which parts are already downloaded.
            let (download_video, download_thumbnail) =
                check_downloads(client.clone(), video).await?;
            if !download_video && !download_thumbnail {
                // All downloads have completed successfully. Note that
                // This is the only branch that has the ability to return
                // None, signaling reconciliation is complete.
                return determine_download_success_action(client, video).await;
            }
            // Create the download pod, downloading only the requested parts.
            return Ok(Some(VideoAction::CreateDownloadPod(DownloadPodOptions {
                download_video,
                download_thumbnail,
            })));
        }
    };

    // Given that the download pod exists, the next action will
    // be dictated by another function for organization's sake.
    determine_download_pod_action(client, video, pod).await
}

/// needs_pending returns true if the video resource
/// requires a status update to set the phase to Pending.
/// This should be the first action for any managed resource.
fn needs_pending(video: &Video) -> bool {
    video.status.is_none() || video.status.as_ref().unwrap().phase.is_none()
}

/// The "read" phase of the reconciliation loop.
async fn determine_action(client: Client, video: &Video) -> Result<VideoAction, Error> {
    if video.meta().deletion_timestamp.is_some() {
        // We only want to garbage collect child resources.
        return Ok(VideoAction::Delete);
    };

    // Make sure the status object exists with a phase.
    // If not, create it and set the phase to Pending.
    // This allows us to access the status and phase
    // fields without having to check for None values.
    if needs_pending(video) {
        // The resource first appeared to the control.
        return Ok(VideoAction::SetPending);
    }

    // Check if the video and/or thumbnail need to
    // be downloaded. Both of these operations must
    // occur behind a VPN connection, so we will do
    // both tasks in the same pod.
    if let Some(action) = determine_download_action(client, video).await? {
        return Ok(action);
    };

    //
    // Any additional actions that occur after the
    // video is fully downloaded will go here.

    // Everything is done and there is nothing to do.
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
    #[error("Kubernetes error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },

    /// Any non-credentials errors from `rust-s3` crate
    #[error("S3 service error: {source}")]
    S3Error {
        #[from]
        source: s3::error::S3Error,
    },

    /// Any credentials errors from `rust-s3` crate
    #[error("S3 credentials error: {source}")]
    S3CredentialsError {
        #[from]
        source: awscreds::error::CredentialsError,
    },

    /// Error in user input or Video resource definition, typically missing fields.
    #[error("Invalid Video CRD: {0}")]
    UserInputError(String),

    /// Video status.phase value does not match any known phase.
    #[error("Invalid Video status.phase: {0}")]
    InvalidPhase(String),

    /// Generic error based on a string description
    #[error("error: {0}")]
    GenericError(String),
}
