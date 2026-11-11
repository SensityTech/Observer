//! The integration contract as it ships, for the panel's documentation: `resource/CONTRACT.md`
//! itself, the example resource that follows it, and what the bridge's script raises and exports.
//! The contract is written once, in `resource/`: nothing here says it again.

use crate::bridge;

/// `resource/CONTRACT.md`, in Markdown.
pub const TEXT: &str = include_str!("../../../resource/CONTRACT.md");

/// The example resource, file by file. Each is named from `resource/`, as the contract's own
/// links are: the one it gives to `observer_example` leads to these.
pub const EXAMPLE: [(&str, &str); 2] = [
    (
        "observer_example/fxmanifest.lua",
        include_str!("../../../resource/observer_example/fxmanifest.lua"),
    ),
    (
        "observer_example/server.lua",
        include_str!("../../../resource/observer_example/server.lua"),
    ),
];

/// What every event of the contract starts with.
pub const EVENT_PREFIX: &str = "observer:";

/// An export of the bridge, as its script declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub name: &'static str,
    pub parameters: Vec<&'static str>,
}

/// Every server event the bridge raises, as resources hear it (`observer:shuttingDown`), read
/// from its script: those Observer has it raise by name, then those it raises on its own.
pub fn events() -> Vec<String> {
    events_of(bridge::script())
}

/// Every export of the bridge, in the order its script declares them.
pub fn exports() -> Vec<Export> {
    exports_of(bridge::script())
}

fn events_of(script: &str) -> Vec<String> {
    let table = script
        .split_once("local EVENTS = {\n")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map_or("", |(table, _)| table);
    let named = table
        .lines()
        .filter_map(|line| line.trim().strip_suffix(" = true,"));
    // `TriggerEvent('observer:' .. name` raises the named ones: it holds no name of its own.
    let own = script
        .split("TriggerEvent('")
        .skip(1)
        .filter_map(|rest| rest.split_once('\'')?.0.strip_prefix(EVENT_PREFIX))
        .filter(|name| !name.is_empty());
    let mut events: Vec<String> = Vec::new();
    for name in named.chain(own) {
        let event = format!("{EVENT_PREFIX}{name}");
        if !events.contains(&event) {
            events.push(event);
        }
    }
    events
}

fn exports_of(script: &'static str) -> Vec<Export> {
    script
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.strip_prefix("exports('")?.split_once('\'')?;
            let (parameters, _) = rest.split_once("function(")?.1.split_once(')')?;
            Some(Export {
                name,
                parameters: parameters
                    .split(',')
                    .map(str::trim)
                    .filter(|parameter| !parameter.is_empty())
                    .collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_are_those_the_script_names_and_those_it_raises_on_its_own() {
        let script = "local EVENTS = {\n    shuttingDown = true,\n    startChanges = true,\n}\n\
            local OTHER = {\n    verdict = true,\n}\n\
            TriggerEvent('observer:playerRefused', refusal.event)\n\
            TriggerEvent('observer:playerRefused', again)\n\
            TriggerEvent('observer:' .. command.event, command.payload)\n\
            TriggerEvent(event[1], event[2])\n\
            TriggerEvent('txAdmin:events:scheduledRestart', {})\n";

        assert_eq!(
            events_of(script),
            [
                "observer:shuttingDown",
                "observer:startChanges",
                "observer:playerRefused"
            ]
        );
    }

    #[test]
    fn the_exports_come_with_their_parameters() {
        let script = "exports('getContractVersion', function()\n    return 1\nend)\n\
            exports('setPlayerLabel', function(player, label)\nend)\n\
            local function exports_of() end\n";

        assert_eq!(
            exports_of(script),
            [
                Export {
                    name: "getContractVersion",
                    parameters: vec![]
                },
                Export {
                    name: "setPlayerLabel",
                    parameters: vec!["player", "label"]
                },
            ]
        );
    }

    #[test]
    fn the_bridge_raises_events_and_has_exports() {
        let events = events();
        let exports = exports();

        // One Observer has it raise by name, one it raises on its own.
        for event in ["observer:shuttingDown", "observer:playerBanned"] {
            assert!(events.iter().any(|raised| raised == event), "{event}");
        }
        assert!(exports.iter().any(|export| export.name == "getContractVersion"));
    }

    #[test]
    fn the_contract_gives_every_event_of_the_bridge_a_section_and_says_when_it_comes() {
        for event in events() {
            assert!(
                TEXT.contains(&format!("\n### `{event}`\n")),
                "CONTRACT.md has no section for {event}"
            );
            assert!(
                TEXT.contains(&format!("\n| `{event}` | ")),
                "no table of CONTRACT.md says when {event} comes"
            );
        }
    }

    #[test]
    fn the_contract_has_no_section_for_an_event_the_bridge_does_not_raise() {
        let events = events();

        for documented in TEXT
            .lines()
            .filter_map(|line| line.strip_prefix("### `")?.strip_suffix('`'))
            .filter(|heading| heading.starts_with(EVENT_PREFIX))
        {
            assert!(
                events.iter().any(|event| event == documented),
                "the bridge does not raise {documented}"
            );
        }
    }

    #[test]
    fn the_contract_shows_every_export_of_the_bridge_called() {
        for export in exports() {
            assert!(
                TEXT.contains(&format!("]:{}(", export.name)),
                "CONTRACT.md does not show {} called",
                export.name
            );
        }
    }

    #[test]
    fn the_example_resource_ships_whole_and_listens_to_every_event() {
        let [(manifest, declared), (script, text)] = EXAMPLE;

        assert_eq!(manifest, "observer_example/fxmanifest.lua");
        assert_eq!(script, "observer_example/server.lua");
        assert!(
            TEXT.contains("](observer_example)"),
            "CONTRACT.md links to the example"
        );
        assert!(declared.contains("server_script 'server.lua'"));
        for event in events() {
            assert!(
                text.contains(&format!("AddEventHandler('{event}'")),
                "the example does not listen to {event}"
            );
        }
    }
}
