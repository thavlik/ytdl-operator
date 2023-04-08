use std::fs;
use kube::CustomResourceExt;
use ytdl_types::*;

fn main() {
    let _ = fs::create_dir("../crds");
    fs::write("../crds/ytdl.beebs.dev_contentstorage_crd.yaml", serde_yaml::to_string(&ContentStorage::crd()).unwrap()).unwrap();
    fs::write("../crds/ytdl.beebs.dev_metadatatarget_crd.yaml", serde_yaml::to_string(&MetadataTarget::crd()).unwrap()).unwrap();
    fs::write("../crds/ytdl.beebs.dev_download_crd.yaml", serde_yaml::to_string(&Download::crd()).unwrap()).unwrap();
    fs::write("../crds/ytdl.beebs.dev_downloadchildprocess_crd.yaml", serde_yaml::to_string(&DownloadChildProcess::crd()).unwrap()).unwrap();
}

