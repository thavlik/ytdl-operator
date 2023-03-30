use std::fs;
use kube::CustomResourceExt;
use ytdl_types::*;

fn main() {
    let _ = fs::create_dir("../crds");
    fs::write("../crds/ytdl.beebs.dev_executor_crd.yaml", serde_yaml::to_string(&Executor::crd()).unwrap()).unwrap();
    fs::write("../crds/ytdl.beebs.dev_download_crd.yaml", serde_yaml::to_string(&Download::crd()).unwrap()).unwrap();
}

