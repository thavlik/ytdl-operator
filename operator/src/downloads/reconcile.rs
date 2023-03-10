use futures::stream::StreamExt;
use k8s_openapi::api::core::v1::{ConfigMap, Pod, PodStatus};
use kube::Resource;
use kube::ResourceExt;
use kube::{
    api::ListParams, client::Client, runtime::controller::Action, runtime::Controller, Api,
};
use std::sync::Arc;
use tokio::time::Duration;

use super::action::{self, ProgressOptions};
use ytdl_common::{
    check_pod_scheduling_error, create_executor, get_download_phase, get_executor,
    get_executor_service_account_name, Entity, Error, IMMEDIATELY, INFO_JSONL_KEY,
};
use ytdl_types::{Download, DownloadPhase, ExecutorPhase};

pub async fn main() {
    println!("Initializing Download controller...");

    // First, a Kubernetes client must be obtained using the `kube` crate
    // The client will later be moved to the custom controller
    let kubernetes_client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");

    // The executor service account name is required for the query pod
    // to create its ConfigMap and child Executors.
    let service_account_name = get_executor_service_account_name()
        .expect("Expected a valid executor service account name.");

    // Preparation of resources used by the `kube_runtime::Controller`
    let crd_api: Api<Download> = Api::all(kubernetes_client.clone());
    let context: Arc<ContextData> = Arc::new(ContextData::new(
        kubernetes_client.clone(),
        service_account_name,
    ));

    // The controller comes from the `kube_runtime` crate and manages the reconciliation process.
    // It requires the following information:
    // - `kube::Api<T>` this controller "owns". In this case, `T = Download`, as this controller owns the `Download` resource,
    // - `kube::api::ListParams` to select the `Download` resources with. Can be used for Download filtering `Download` resources before reconciliation,
    // - `reconcile` function with reconciliation logic to be called each time a resource of `Download` kind is created/updated/deleted,
    // - `on_error` function to call whenever reconciliation fails.
    println!("Starting Download controller...");
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
    service_account_name: String,
}

impl ContextData {
    /// Constructs a new instance of ContextData.
    ///
    /// # Arguments:
    /// - `client`: A Kubernetes client to make Kubernetes REST API requests with. Resources
    /// will be created and deleted with this client.
    pub fn new(client: Client, service_account_name: String) -> Self {
        ContextData {
            client,
            service_account_name,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct QueryFailureOptions {
    message: String,
    recreate: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum ReconcileAction {
    // The resource first appeared to the controller and requires
    // its phase to be set to "Pending" to indicate that reconciliation
    // is in progress.
    Pending,

    // Delete all child resources.
    Delete,

    CreateQueryPod,

    DeleteQueryPod,

    QueryFailure(QueryFailureOptions),

    QueryProgress(ProgressOptions),

    CreateExecutor(Entity),

    DownloadProgress { succeeded: usize, total: usize },

    Succeeded,

    /*
    // Create the pod to download the video and/or thumbnail. Subsequent
    // reconciliations will update the Download's status to reflect the
    // progress of the download.
    Query(QueryPodOptions),

    // Delete the download pod. This is done when the Download resource is
    // deleted and when the download pod needs to be deleted to proceed
    // with reconciliation.

    // The download pod is still downloading the video and/or thumbnail.

    // Download pod has finished downloading the video and/or thumbnail.

    // Download pod has failed with an error message.
    Failure(FailureOptions),
    */
    // Nothing to do (reconciliation successful)
    NoOp,
}

/// Main reconciliation loop for the `Download` resource.
async fn reconcile(instance: Arc<Download>, context: Arc<ContextData>) -> Result<Action, Error> {
    // The `Client` is shared -> a clone from the reference is obtained.
    let client: Client = context.client.clone();

    let namespace: String = match instance.namespace() {
        None => {
            // If there is no namespace to deploy to defined, reconciliation ends with an error immediately.
            return Err(Error::UserInputError(
                "Expected Download resource to be namespaced. Can't deploy to an unknown namespace."
                    .to_owned(),
            ));
        }
        // If namespace is known, proceed. In a more advanced version of the operator, perhaps
        // the namespace could be checked for existence first.
        Some(namespace) => namespace,
    };

    // Name of the Download resource is used to name the subresources as well.
    let name = instance.name_any();

    // Read phase of the reconciliation loop.
    let action = determine_action(client.clone(), &instance).await?;

    if action != ReconcileAction::NoOp {
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
        ReconcileAction::Pending => {
            // Update the status of the resource to reflect that reconciliation is in progress.
            action::pending(client, &name, &namespace, &instance).await?;

            // Requeue the resource to be immediately reconciled again.
            Ok(Action::requeue(IMMEDIATELY))
        }
        ReconcileAction::Delete => {
            // Delete the query pod.
            action::delete_query_pod(client.clone(), &name, &namespace).await?;

            // Delete all of the child Executors.
            // Executors are garbage collected using owner references.
            //action::delete_executors(client.clone(), &name, &namespace).await?;

            // Once everything is successfully deleted, remove the finalizer to make
            // it possible for Kubernetes to delete the `Download` resource.
            action::finalizer::delete(client, &name, &namespace).await?;

            // No need to requeue the resource when it's being deleted.
            Ok(Action::await_change())
        }
        ReconcileAction::DeleteQueryPod => {
            // Delete just the query pod.
            action::delete_query_pod(client, &name, &namespace).await?;

            // Requeue immediately to proceed with reconciliation.
            Ok(Action::requeue(IMMEDIATELY))
        }
        ReconcileAction::CreateQueryPod => {
            // Apply the finalizer first. This way the Download resource
            // won't be deleted before the query pod is deleted.
            let instance = action::finalizer::add(client.clone(), &name, &namespace).await?;

            // Create the executor pod that queries the info jsonl and
            // creates child Executor resources for each entity.
            action::create_query_pod(
                client.clone(),
                &name,
                &namespace,
                &instance,
                context.service_account_name.clone(),
            )
            .await?;

            // Update the Download's status to reflect the starting query.
            action::query_starting(client, &name, &namespace, &instance).await?;

            // Requeue after a short delay to give the pod time to schedule/start.
            Ok(Action::requeue(Duration::from_secs(5)))
        }
        ReconcileAction::QueryFailure(options) => {
            // Update the Download's status to include the failure message.
            action::query_failure(
                client.clone(),
                &name,
                &namespace,
                &instance,
                options.message,
            )
            .await?;

            if options.recreate {
                // Delete the query pod so it can be recreated.
                action::delete_query_pod(client, &name, &namespace).await?;
                // Display the error message for a short while before
                // requeueing as a form of back-off.
                return Ok(Action::requeue(Duration::from_secs(5)));
            }

            // Don't requeue until the resource is changed.
            Ok(Action::await_change())
        }
        ReconcileAction::QueryProgress(opts) => {
            match opts.start_time {
                // Update the Download's status to reflect the progress of the query.
                Some(start_time) => {
                    action::query_progress(client, &name, &namespace, &instance, start_time)
                        .await?
                }
                // Query pod start time is not yet available.
                None => {
                    action::query_starting(client, &name, &namespace, &instance).await?
                }
            }
            // Requeue after a short delay to check query progress again.
            Ok(Action::requeue(Duration::from_secs(3)))
        }
        ReconcileAction::DownloadProgress { succeeded, total } => {
            // Update the status object to show download progress.
            action::download_progress(
                client,
                &name,
                &namespace,
                &instance,
                succeeded,
                total,
            )
            .await?;

            // Requeue after a short delay to check download progress again.
            Ok(Action::requeue(Duration::from_secs(3)))
        }
        ReconcileAction::CreateExecutor(entity) => {
            // Apply the finalizer first. This way the Download resource
            // won't be deleted before the child Executor is deleted.
            let instance = action::finalizer::add(client.clone(), &name, &namespace).await?;

            // Create the child Executor from the entity.
            create_executor(client, &instance, entity.id, entity.metadata).await?;

            // Requeue without delay as there may be other Executors to create.
            Ok(Action::requeue(IMMEDIATELY))
        }
        ReconcileAction::Succeeded => {
            // Update the status object to show that the downloads are complete.
            action::succeeded(client, &name, &namespace, &instance).await?;

            // Requeue only when the resource changes.
            Ok(Action::await_change())
        }
        ReconcileAction::NoOp => {
            // Nothing to do (resource is fully reconciled).
            Ok(Action::await_change())
        }
    }
}

/// needs_pending returns true if the `Download` resource
/// requires a status update to set the phase to Pending.
/// This should be the first action for any managed resource.
fn needs_pending(instance: &Download) -> bool {
    instance.status.is_none() || instance.status.as_ref().unwrap().phase.is_none()
}

/// Returns the ConfigMap that stores the info jsonl for the query.
async fn get_metadata_configmap(
    client: Client,
    instance: &Download,
) -> Result<Option<ConfigMap>, Error> {
    let cm_api: Api<ConfigMap> = Api::namespaced(client, &instance.namespace().unwrap());
    match cm_api.get(&instance.name_any()).await {
        Ok(cm) => Ok(Some(cm)),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Returns the query pod if it exists, or None if it does not.
async fn get_query_pod(client: Client, instance: &Download) -> Result<Option<Pod>, Error> {
    let pod_api: Api<Pod> = Api::namespaced(client, &instance.namespace().unwrap());
    match pod_api.get(&instance.name_any()).await {
        Ok(pod) => Ok(Some(pod)),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Determine the action given that query pod exists.
async fn determine_query_pod_action(
    client: Client,
    instance: &Download,
    pod: Pod,
) -> Result<ReconcileAction, Error> {
    let status: &PodStatus = pod
        .status
        .as_ref()
        .ok_or_else(|| Error::UnknownError("query pod has no status".to_owned()))?;
    let phase: &str = status
        .phase
        .as_ref()
        .ok_or_else(|| Error::UnknownError("query pod has no phase".to_owned()))?;
    match phase {
        "Pending" => {
            // Query pod is not yet started.
            if let Some(message) = check_pod_scheduling_error(&status) {
                // There was some kind of scheduling error. We don't
                // want to recreate the pod in this case, only report.
                return Ok(ReconcileAction::QueryFailure(QueryFailureOptions {
                    message,
                    recreate: false,
                }));
            }
            // Query pod is Pending without error.
            // Mark the Executor phase as being in-progress.
            Ok(ReconcileAction::QueryProgress(ProgressOptions {
                start_time: None,
            }))
        }
        "Running" => {
            // Query is in progress.
            // TODO: report verbose download statistics.
            Ok(ReconcileAction::QueryProgress(ProgressOptions {
                start_time: pod.creation_timestamp(),
            }))
        }
        "Succeeded" => {
            // Make sure the metadata ConfigMap exists. If it does not,
            // this is an error condition as the pod completed without
            // creating it. This should never happen and is more of a
            // sanity check than anything.
            if get_metadata_configmap(client.clone(), instance)
                .await?
                .is_none()
            {
                return Ok(ReconcileAction::QueryFailure(QueryFailureOptions {
                    message: "query pod completed without creating metadata ConfigMap".to_owned(),
                    // We want the user to see this error, so don't recreate.
                    recreate: false,
                }));
            }
            // Query is completed. Delete the query pod and requeue.
            Ok(ReconcileAction::DeleteQueryPod)
        }
        _ => {
            // Report error, delete pod, and re-create.
            // TODO: find way to extract a verbose error message from the pod.
            let message = format!("query pod is in phase {}", phase);
            Ok(ReconcileAction::QueryFailure(QueryFailureOptions {
                message,
                recreate: true,
            }))
        }
    }
}

/// Determines the action given that the metadata ConfigMap
/// does not exist, signifying that the query has not yet
/// completed.
async fn determine_query_action(
    client: Client,
    instance: &Download,
) -> Result<ReconcileAction, Error> {
    // Check to see if query pod exists.
    match get_query_pod(client.clone(), instance).await? {
        // Pod exists, action depends on the pod's status.
        Some(pod) => determine_query_pod_action(client, instance, pod).await,
        // Pod does not exist, create it.
        None => Ok(ReconcileAction::CreateQueryPod),
    }
}

fn parse_id(line: &str) -> Result<String, Error> {
    // Parse the video metadata json.
    let info: serde_json::Value = serde_json::from_str(line)?;

    // Get the ID field. This is used to name the Executor.
    Ok(info
        .get("id")
        .ok_or_else(|| Error::UnknownError("info.jsonl line has no id".to_owned()))?
        .as_str()
        .ok_or_else(|| Error::UnknownError("info.jsonl id is not a string".to_owned()))?
        .to_owned())
}

async fn determine_executor_action(
    client: Client,
    instance: &Download,
    info_jsonl: &str,
) -> Result<ReconcileAction, Error> {
    // Keep track of child Executor population status.
    let mut total = 0;
    let mut succeeded = 0;

    // Reconcile the Executors for each line in info.jsonl.
    for line in info_jsonl.split('\n') {
        // Attempt to parse the line into json. If it fails,
        // skip it and go to the next line.
        let id = match parse_id(line) {
            Ok(v) => v,
            Err(_) => {
                // Skip this line if we can't parse it.
                // Could be an error message or something.
                continue;
            }
        };

        // Get the Executor for the entity.
        let executor_name = format!("{}-{}", instance.name_any(), id);
        let executor = match get_executor(
            client.clone(),
            &executor_name,
            instance.namespace().as_ref().unwrap(),
        )
        .await
        {
            Ok(Some(executor)) => executor,
            Ok(None) => {
                // Executor does not exist, create it.
                return Ok(ReconcileAction::CreateExecutor(Entity {
                    id,
                    metadata: line.to_owned(),
                }));
            }
            Err(e) => {
                return Err(e);
            }
        };

        // Increment the total number of Executors.
        total += 1;

        // Check the status of the Executor.
        match executor.status {
            Some(ref status) => match status.phase {
                Some(ref phase) => {
                    if phase == ExecutorPhase::Succeeded.to_str() {
                        // Increment the number of succeeded Executors.
                        succeeded += 1;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    if succeeded != total {
        // Not all Executors have succeeded, report the progress.
        return Ok(ReconcileAction::DownloadProgress { succeeded, total });
    }
    match get_download_phase(instance)? {
        // Nothing to do, we're already in the Succeeded phase.
        DownloadPhase::Succeeded => Ok(ReconcileAction::NoOp),
        // Mark the phase as Succeeded.
        _ => Ok(ReconcileAction::Succeeded),
    }
}

/// The "read" phase of the reconciliation loop.
async fn determine_action(client: Client, instance: &Download) -> Result<ReconcileAction, Error> {
    if instance.meta().deletion_timestamp.is_some() {
        // We only want to garbage collect child resources.
        return Ok(ReconcileAction::Delete);
    };

    // Make sure the status object exists with a phase.
    // If not, create it and set the phase to Pending.
    // This allows us to access the status and phase
    // fields without having to check for None values.
    if needs_pending(instance) {
        // The resource first appeared to the control.
        return Ok(ReconcileAction::Pending);
    }

    // First step is to reconcile the metadata ConfigMap.
    let metadata: ConfigMap = match get_metadata_configmap(client.clone(), instance).await {
        // ConfigMap exists. All we need to do now is manage
        // all of the child Executors, one for each line of
        // the payload.
        Ok(Some(cm)) => cm,
        // No metadata ConfigMap exists. This means the query
        // has not completed yet.
        Ok(None) => {
            return determine_query_action(client, instance).await;
        }
        // Unable to access ConfigMap.
        Err(e) => {
            return Err(e);
        }
    };

    // Get the contents of info.jsonl from the ConfigMap.
    let data = metadata
        .data
        .ok_or_else(|| Error::UnknownError("metadata ConfigMap has no data".to_owned()))?;
    let info_jsonl = data
        .get(INFO_JSONL_KEY)
        .ok_or_else(|| Error::UnknownError("metadata ConfigMap has no info.jsonl".to_owned()))?;

    // The rest of this controller and the query executor
    // itself share code for creating child Executors from
    // `youtube-dl -j` jsonl output. This allows downloads
    // to start before the query is finished, which may take
    // a long time for huge channels or playlists.
    determine_executor_action(client, instance, info_jsonl).await
}

/// Actions to be taken when a reconciliation fails - for whatever reason.
/// Prints out the error to `stderr` and requeues the resource for another reconciliation after
/// five seconds.
///
/// # Arguments
/// - `instance`: The erroneous resource.
/// - `error`: A reference to the `kube::Error` that occurred during reconciliation.
/// - `_context`: Unused argument. Context Data "injected" automatically by kube-rs.
fn on_error(instance: Arc<Download>, error: &Error, _context: Arc<ContextData>) -> Action {
    eprintln!("Reconciliation error:\n{:?}.\n{:?}", error, instance);
    Action::requeue(Duration::from_secs(5))
}
