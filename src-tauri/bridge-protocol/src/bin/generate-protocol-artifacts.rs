use std::fs;
use std::path::PathBuf;

fn main() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for artifact in bridge_protocol::tsgen::artifacts() {
        let target = repository_root.join(artifact.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create artifact directory");
        }
        fs::write(&target, artifact.content).expect("write artifact");
        println!("wrote {}", artifact.path);
    }
}
