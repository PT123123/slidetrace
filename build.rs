fn main() {
    println!("cargo:rerun-if-changed=ui");
    slint_build::compile("ui/main.slint").expect("Slint UI 编译失败（ui/main.slint）");
}
