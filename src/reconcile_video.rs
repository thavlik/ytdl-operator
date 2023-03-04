use futures::stream::StreamExt;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::Duration;
use k8s_openapi::api::core::v1::{Pod, PodStatus, Secret};
use kube::Resource;
use kube::ResourceExt;
use kube::{
    api::ListParams,
    client::Client,
    runtime::controller::Action,
    runtime::Controller,
    Api,
};
use awsregion::Region;
use s3::bucket::Bucket;
use s3::creds::Credentials;

use crate::crd::{S3OutputSpec, Video, VideoPhase};
use crate::video::{self, DownloadPodOptions, FailureOptions, ProgressOptions};

const DEFAULT_REGION: &str = "us-east-1";
const DEFAULT_TEMPLATE: &str = "%(id)s.%(ext)s";

pub async fn reconcile_video_main() {
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
    Pending,

    // Create the pod to download the video and/or thumbnail. Subsequent
    // reconciliations will update the Video's status to reflect the
    // progress of the download.
    CreateDownloadPod(DownloadPodOptions),

    // Delete the download pod. This is done when the Video resource is
    // deleted and when the download pod needs to be deleted to proceed
    // with reconciliation.
    DeleteDownloadPod,

    // The download pod is still downloading the video and/or thumbnail.
    Progress(ProgressOptions),

    // Download pod has finished downloading the video and/or thumbnail.
    Succeeded,

    // Download pod has failed with an error message.
    Failure(FailureOptions),

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
        VideoAction::Pending => {
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
        VideoAction::DeleteDownloadPod => {
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
        VideoAction::Progress(options) => {
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
        VideoAction::Succeeded => {
            // Update the status of the resource to reflect download completion.
            video::success(client.clone(), &name, &namespace, video.as_ref()).await?;

            // Delete the download pod before the finalizer is removed.
            video::delete_download_pod(client.clone(), &name, &namespace).await?;

            // Remove the finalizer now that the download pod is gone.
            video::finalizer::delete(client, &name, &namespace).await?;

            // Requeue immediately.
            Ok(Action::requeue(Duration::ZERO))
        }
        VideoAction::Failure(options) => {
            // Update the status of the resource to communicate the error.
            video::failure(client.clone(), &name, &namespace, video.as_ref(), options).await?;

            // Delete the download pod so it can be recreated.
            video::delete_download_pod(client, &name, &namespace).await?;

            // Requeue immediately.
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
        return Ok(false);
    }
    Ok(head.content_length.unwrap_or(0) > 0)
}

/// Returns the output key given the template and the
/// video's metadata. This requires deserializing the
/// metadata and iterating over its contents to replace
/// the template variables with their values.
fn template_key(metadata: &serde_json::Value, template: &str) -> Result<String, Error> {
    // Parse the metadata into a generic json object.
    let metadata = metadata
        .as_object()
        .ok_or_else(|| Error::UserInputError("metadata must be a json object".to_owned()))?;
    // Iterate over the key-value pairs and replace the template variables.
    let mut result = template.to_owned();
    for (key, value) in metadata {
        // Default to an empty string if the value is not a string.
        let value = value.as_str().unwrap_or("");
        // Replace the template variable with the value.
        result = result.replace(&format!("%({})s", key), value);
    }
    Ok(result)
}

/// Returns true if the video needs to be downloaded.
async fn needs_video_download(
    client: Client,
    metadata: &serde_json::Value,
    video: &Video,
) -> Result<bool, Error> {
    let (bucket, template) = match get_video_output(client, video).await? {
        // Resource is requesting video output.
        Some(v) => v,
        // Resource is not configured to output video.
        // This would be the case if the user only wants
        // to download metadata and thumbnail.
        None => return Ok(false),
    };
    // Conver the template into the actual S3 object key.
    let key = template_key(metadata, &template)?;
    // Check if the object exists and is not empty.
    bucket_has_obj(bucket, &key).await
}

/// Returns true if the thumbnail needs to be downloaded.
async fn needs_thumbnail_download(
    client: Client,
    metadata: &serde_json::Value,
    video: &Video,
) -> Result<bool, Error> {
    let (bucket, template) = match get_thumbnail_output(client, video).await? {
        // Resource is requesting thumbnail output.
        Some(v) => v,
        // Resource is not requesting thumbnail output.
        None => return Ok(false),
    };
    // Convert the template into the actual S3 object key.
    let key = template_key(metadata, &template)?;
    // Check if the object exists and is not empty.
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
    let metadata: serde_json::Value = video.spec.metadata.parse()?;
    let result = tokio::join!(
        needs_video_download(client.clone(), &metadata, video),
        needs_thumbnail_download(client, &metadata, video),
    );
    let download_video = result.0?;
    let download_thumbnail = result.1?;
    Ok((download_video, download_thumbnail))
}

/// Returns the secret value for the given key.
/// This requires an allocation because it's unclear
/// how to pass &ByteString into std::str::from_utf8
/// and still satisfy the borrow checker.
fn get_secret_value(secret: &Secret, key: &str) -> Result<Option<String>, Error> {
    Ok(match secret.data {
        Some(ref data) => match data.get(key) {
            Some(s) => Some(serde_json::to_string(s)?),
            None => None,
        },
        None => None,
    })
}

/// Returns the S3 credentials for the given S3OutputSpec.
async fn get_s3_creds(
    client: Client,
    namespace: &str,
    spec: &S3OutputSpec,
) -> Result<Credentials, Error> {
    let api: Api<Secret> = Api::namespaced(client, namespace);
    let secret: Secret = api.get(&spec.secret).await?;
    let access_key_id = get_secret_value(&secret, "access_key_id")?;
    let secret_access_key = get_secret_value(&secret, "secret_access_key")?;
    Ok(Credentials::new(
        access_key_id.as_deref(),
        secret_access_key.as_deref(),
        None, // security token
        None, // session token
        None, // profile
    )?)
}

/// Returns the S3 Region object for the given S3OutputSpec.
fn get_s3_region(spec: &S3OutputSpec) -> Result<Region, Error> {
    let region = match spec.region.as_ref() {
        Some(region) => region.to_owned(),
        None => DEFAULT_REGION.to_owned(),
    };
    Ok(match spec.endpoint.as_ref() {
        // Custom endpoint support (e.g. https://nyc3.digitaloceanspaces.com)
        Some(endpoint) => Region::Custom {
            region,
            endpoint: endpoint.clone(),
        },
        // The Region object is based solely on the region name.
        None => region.parse()?,
    })
}

/// Returns the S3 Bucket and key template for the given S3OutputSpec.
async fn output_from_spec(
    client: Client,
    namespace: &str,
    spec: &S3OutputSpec,
) -> Result<(Bucket, String), Error> {
    let region = get_s3_region(spec)?;
    let credentials = get_s3_creds(client, namespace, spec).await?;
    let bucket = Bucket::new(&spec.bucket, region, credentials)?;
    let template = match spec.template {
        Some(ref template) => template.clone(),
        None => DEFAULT_TEMPLATE.to_owned(),
    };
    Ok((bucket, template))
}

/// Returns the Bucket to be used for video file storage.
async fn get_video_output(
    client: Client,
    video: &Video,
) -> Result<Option<(Bucket, String)>, Error> {
    match video.spec.output.video.as_ref().unwrap().s3.as_ref() {
        Some(spec) => Ok(Some(
            output_from_spec(client, video.namespace().as_ref().unwrap(), spec).await?,
        )),
        None => Ok(None),
    }
}

/// Returns the Bucket to be used for thumbnail storage.
async fn get_thumbnail_output(
    client: Client,
    video: &Video,
) -> Result<Option<(Bucket, String)>, Error> {
    match video.spec.output.thumbnail.as_ref().unwrap().s3.as_ref() {
        Some(spec) => Ok(Some(
            output_from_spec(client, video.namespace().as_ref().unwrap(), spec).await?,
        )),
        None => Ok(None),
    }
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
        return Ok(Some(VideoAction::Succeeded));
    }
    match get_download_pod(client, video).await? {
        // Garbage collect the download pod. Given that
        // the Delete action is invoked after the pod
        // succeeds, this branch *shouldn't* be reached,
        // but for safety we handle it anyway.
        Some(_) => Ok(Some(VideoAction::DeleteDownloadPod)),
        // Do nothing and proceed with reconciliation.
        None => Ok(None),
    }
}

/// Determines the action to take given that the download pod
/// exists and we need to check its status.
async fn determine_download_pod_action(
    client: Client,
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
                        return Ok(Some(VideoAction::Failure(FailureOptions { message })));
                    }
                }
            }
            // Download pod is Pending without error.
            // Mark the Video phase as being in-progress.
            Ok(Some(VideoAction::Progress(ProgressOptions {
                start_time: None,
            })))
        }
        "Running" => {
            // Download is in progress.
            // TODO: report verbose download statistics.
            Ok(Some(VideoAction::Progress(ProgressOptions {
                start_time: pod.creation_timestamp(),
            })))
        }
        "Succeeded" => {
            // Download is completed.
            Ok(Some(VideoAction::Succeeded))
        }
        _ => {
            // Report error, delete pod, and re-create.
            // TODO: find way to extract a verbose error message from the pod.
            let message = format!("pod is in phase {}", phase);
            Ok(Some(VideoAction::Failure(FailureOptions { message })))
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
    match get_download_pod(client.clone(), video).await? {
        // Download pod exists, no reason to check storage
        // as the results of `check_downloads` are cached
        // in the pod's spec.
        Some(pod) => determine_download_pod_action(client, pod).await,
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
            Ok(Some(VideoAction::CreateDownloadPod(DownloadPodOptions {
                download_video,
                download_thumbnail,
            })))
        }
    }
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
        return Ok(VideoAction::DeleteDownloadPod);
    };

    // Make sure the status object exists with a phase.
    // If not, create it and set the phase to Pending.
    // This allows us to access the status and phase
    // fields without having to check for None values.
    if needs_pending(video) {
        // The resource first appeared to the control.
        return Ok(VideoAction::Pending);
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

    /// Error converting a string to UTF-8
    #[error("UTF-8 error: {source}")]
    Utf8Error {
        #[from]
        source: std::str::Utf8Error,
    },

    /// Serde json decode error
    #[error("decode json error: {source}")]
    JSONError {
        #[from]
        source: serde_json::Error,
    },
}
