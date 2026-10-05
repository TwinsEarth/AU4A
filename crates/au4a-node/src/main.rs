//! AU4A 节点二进制 —— v1.x 骨架版入口。
fn main() {
    let version = include_str!("../../../VERSION").trim();
    let checks = au4a_node::self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    println!("au4a-node {version} (skeleton) — {passed}/{} checks passed", checks.len());
    std::process::exit(if au4a_core::all_passed(&checks) { 0 } else { 1 });
}
