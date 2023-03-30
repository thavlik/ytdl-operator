use const_format::concatcp;
use k8s_openapi::{
    api::core::v1::{
        Capabilities, Container, EmptyDirVolumeSource, EnvVar, EnvVarSource, Pod, PodSpec,
        SecretKeySelector, SecurityContext, Volume, VolumeMount,
    },
    apimachinery::pkg::apis::meta::v1::OwnerReference,
};
use kube::api::ObjectMeta;
use std::collections::BTreeMap;

/// The IP service to use for getting the public IP address.
pub const IP_SERVICE: &str = "https://api.ipify.org";

/// Name of the shared volume, used to share files between
/// containers and detect when the VPN connected. Containers
/// should mount this volume at `SHARED_PATH` and access
/// the initial ip file at `IP_FILE_PATH` to know when the
/// VPN finishes connecting.
pub const SHARED_VOLUME_NAME: &str = "shared";

/// Shared directory path.
pub const SHARED_PATH: &str = "/shared";

/// The file containing the unmasked IP address of the pod.
/// This is written by an init container so the executor
/// knows when the VPN is connected.
pub const IP_FILE_PATH: &str = concatcp!(SHARED_PATH, "/ip");

/// VPN sidecar image. Efforts were made to use a stock
/// image with no modifications, as to maximize the
/// modular nature of the sidecar.
const DEFAULT_VPN_IMAGE: &str = "qmcgaw/gluetun:v3.32.0";

/// Creates the container spec for the VPN sidecar.
pub fn get_vpn_sidecar() -> Container {
    Container {
        name: "vpn".to_owned(),
        image: Some(DEFAULT_VPN_IMAGE.to_owned()),
        image_pull_policy: Some("IfNotPresent".to_owned()),
        security_context: Some(SecurityContext {
            capabilities: Some(Capabilities {
                add: Some(vec!["NET_ADMIN".to_owned()]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        env: Some(vec![
            // TODO: configure gluetun env vars
            // https://github.com/qdm12/gluetun/wiki/
            EnvVar {
                name: "VPN_SERVICE_PROVIDER".to_owned(),
                value: Some("private internet access".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "IP_SERVICE".to_owned(),
                value: Some(IP_SERVICE.to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "OPENVPN_USER".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("pia-creds".to_owned()),
                        key: "username".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            EnvVar {
                name: "OPENVPN_PASSWORD".to_owned(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some("pia-creds".to_owned()),
                        key: "password".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
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
            name: SHARED_VOLUME_NAME.to_owned(),
            mount_path: SHARED_PATH.to_owned(),
            ..VolumeMount::default()
        }]),
        ..Container::default()
    }
}

pub fn masked_pod(
    name: String,
    namespace: String,
    owner_references: Option<Vec<OwnerReference>>,
    service_account_name: String,
    container: Container,
) -> Pod {
    // Add a label to the pod so that we can easily find it.
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert("app".to_owned(), "ytdl".to_owned());

    // The containers have a shared volume mounted at /share
    // that the VPN pod will write a file to when it's ready.
    // This way the executor pod can wait for the VPN to be
    // fully connected before starting any downloads.
    // Kubernetes does not provide robust enough means of
    // ensuring the VPN is connected before starting other
    // containers, so this is the best we can do.
    Pod {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace),
            labels: Some(labels),
            owner_references,
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            // The operator is responsible for managing the lifecycle
            // of this pod, so it should never be restarted or retried.
            restart_policy: Some("Never".to_owned()),
            // The pod needs access to the k8s api so it can retrieve
            // e.g. s3 credentials from the configured Secret resources.
            service_account_name: Some(service_account_name),
            // Create an init container that writes the unmasked public
            // IP to a shared file. This container must complete before
            // the others can start, and this is useful when the executor
            // is trying to figure out the moment the VPN is connected.
            init_containers: Some(vec![get_init_container()]),
            // Main containers will start only after the init container
            // succeeds. Because all containers in a pod share the same
            // networking, connecting to a VPN in a sidecar will connect
            // all other containers as well. The executor will detect
            // the new/masked IP before starting any downloads.
            containers: vec![
                // Each executor will have a VPN sidecar to avoid drawing
                // too much attention from the video service.
                // Kubelet will start the VPN container first. If both
                // images are already available on the node, this should
                // result in less time waiting for the VPN connection.
                get_vpn_sidecar(),
                // Starting the executor container last may reduce VPN
                // connection wait time.
                container,
            ],
            // Create an in-memory volume that allows data to be shared
            // between the containers. The init container will write the
            // unmasked public IP to a file in this volume, and the
            // executor container will use its contents to determine
            // when the VPN is truly connected. This allows for the
            // widest variety of VPN drivers to be used without any
            // need to write custom logic for each to probe readiness.
            volumes: Some(vec![Volume {
                name: SHARED_VOLUME_NAME.to_owned(),
                empty_dir: Some(EmptyDirVolumeSource {
                    ..EmptyDirVolumeSource::default()
                }),
                ..Volume::default()
            }]),
            ..PodSpec::default()
        }),
        ..Pod::default()
    }
}
