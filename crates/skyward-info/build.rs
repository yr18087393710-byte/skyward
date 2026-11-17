//! The release a build names is baked into the library, and a change of it has
//! to rebuild it: cargo does not look at the environment by itself.

fn main() {
    println!("cargo:rerun-if-env-changed=SKYWARD_RELEASE");
}
