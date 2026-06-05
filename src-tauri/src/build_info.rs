include!(concat!(env!("OUT_DIR"), "/build_info.rs"));

pub fn command_banner(command_name: &str) -> String {
    format!("[{command_name}] {BUILD_LABEL}")
}
