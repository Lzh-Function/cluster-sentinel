//! Read-only investigation instructions shared by diagnoses and notifications.

use crate::entity::ManagedEntity;

pub(crate) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn sentinel(arguments: &str) -> String {
    format!("sudo -u sentinel sentinel {arguments}")
}

pub(crate) fn step(location: &str, check: &str, command: &str) -> String {
    format!("{location}で、{check}。\n{command}")
}

pub(crate) fn entity(entity: &ManagedEntity) -> String {
    let name = quote(&format!("{}/{}", entity.entity_type, entity.canonical_name));
    step(
        "Sentinel controller",
        "接続先と各項目の状態、観測時刻を確認してください",
        &sentinel(&format!("entity show {name} --json")),
    )
}

pub(crate) fn observations(entity: &ManagedEntity, probe: &str, check: &str) -> String {
    let name = quote(&format!("{}/{}", entity.entity_type, entity.canonical_name));
    step(
        "Sentinel controller",
        check,
        &sentinel(&format!(
            "entity observations {name} --probe {} --limit 20 --json",
            quote(probe)
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_as_shell_arguments() {
        assert_eq!(quote("node'$(id)"), "'node'\"'\"'$(id)'");
    }
}
