use clap::Command;

use crate::api::schema::panes::TabFloatsToggleProjection;
use crate::api::schema::tabs::{
    tab_id, TabCreateProjection, TabListProjection, TabRenameProjection,
};
use crate::api::schema::{Method, Request, TabTarget};

use super::super::projection::{self, cli_projection, Field, Leaf, ParseError};

cli_projection! {
    pub(crate) TabTargetProjection for TabTarget {
        tab_id => Field::new(String::new()).input(tab_id()),
    }
}

const LEAVES: [Leaf; 7] = [
    Leaf {
        name: "list",
        about: "List tabs",
        id: "cli:tab:list",
        arguments: TabListProjection::arguments,
        method: |matches| TabListProjection::from_matches(matches).map(Method::TabList),
    },
    Leaf {
        name: "create",
        about: "Create a tab",
        id: "cli:tab:create",
        arguments: || {
            let mut arguments = TabCreateProjection::arguments();
            arguments.push(
                clap::Arg::new("no-parent")
                    .long("no-parent")
                    .action(clap::ArgAction::SetTrue)
                    .conflicts_with("parent")
                    .display_order(7),
            );
            arguments
        },
        method: |matches| {
            let mut params = TabCreateProjection::from_matches(matches)?;
            if params.parent_tab_id.is_none() && !matches.get_flag("no-parent") {
                params.caller_pane_id = crate::cli::target::caller_pane_id()
                    .map(|pane_id| crate::cli::normalize_pane_id(&pane_id));
            }
            Ok(Method::TabCreate(params))
        },
    },
    Leaf {
        name: "get",
        about: "Show a tab",
        id: "cli:tab:get",
        arguments: TabTargetProjection::arguments,
        method: |matches| TabTargetProjection::from_matches(matches).map(Method::TabGet),
    },
    Leaf {
        name: "focus",
        about: "Focus a tab",
        id: "cli:tab:focus",
        arguments: TabTargetProjection::arguments,
        method: |matches| TabTargetProjection::from_matches(matches).map(Method::TabFocus),
    },
    Leaf {
        name: "rename",
        about: "Rename a tab",
        id: "cli:tab:rename",
        arguments: TabRenameProjection::arguments,
        method: |matches| TabRenameProjection::from_matches(matches).map(Method::TabRename),
    },
    Leaf {
        name: "close",
        about: "Close a tab",
        id: "cli:tab:close",
        arguments: TabTargetProjection::arguments,
        method: |matches| TabTargetProjection::from_matches(matches).map(Method::TabClose),
    },
    Leaf {
        name: "floats-toggle",
        about: "Toggle or set a tab's float layer",
        id: "cli:tab:floats_toggle",
        arguments: TabFloatsToggleProjection::arguments,
        method: |matches| {
            TabFloatsToggleProjection::from_matches(matches).map(Method::TabFloatsToggle)
        },
    },
];

pub(crate) fn command() -> Command {
    Command::new("tab")
        .about("Manage tabs over the socket API")
        .subcommand_required(true)
        .subcommands(LEAVES.iter().map(Leaf::command))
}

pub(crate) fn request(args: &[String]) -> Result<Request, ParseError> {
    projection::group_request(command(), &LEAVES, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Request, ParseError> {
        request(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn tab_create_records_the_calling_pane_unless_told_otherwise() {
        crate::cli::target::set_test_caller_pane_id(Some("w1:p3"));
        let params = |args: &[&str]| match parse(args).unwrap().method {
            Method::TabCreate(params) => params,
            other => panic!("expected tab.create, got {other:?}"),
        };

        let automatic = params(&["create"]);
        let opted_out = params(&["create", "--no-parent"]);
        let explicit = params(&["create", "--parent", "w1:t2"]);
        let conflicting = parse(&["create", "--parent", "w1:t2", "--no-parent"]);
        crate::cli::target::set_test_caller_pane_id(None);

        assert_eq!(automatic.caller_pane_id.as_deref(), Some("w1:p3"));
        assert_eq!(automatic.parent_tab_id, None);
        assert_eq!(opted_out.caller_pane_id, None);
        assert_eq!(explicit.parent_tab_id.as_deref(), Some("w1:t2"));
        assert_eq!(explicit.caller_pane_id, None);
        assert!(conflicting.is_err());
    }

    #[test]
    fn tab_projection_preserves_requests_defaults_and_maps() {
        for (args, expected) in [
            (
                vec!["list"],
                serde_json::json!({"id":"cli:tab:list","method":"tab.list","params":{}}),
            ),
            (
                vec!["create"],
                serde_json::json!({"id":"cli:tab:create","method":"tab.create","params":{"focus":false}}),
            ),
            (
                vec![
                    "create",
                    "--workspace",
                    "2",
                    "--cwd",
                    "relative",
                    "--label",
                    "first",
                    "--label",
                    "last",
                    "--focus",
                    "--no-focus",
                    "--env",
                    "K=first",
                    "--env",
                    "K=",
                    "--env",
                    "V=a=b",
                ],
                serde_json::json!({"id":"cli:tab:create","method":"tab.create","params":{"workspace_id":"2","cwd":"relative","label":"last","focus":false,"env":{"K":"","V":"a=b"}}}),
            ),
            (
                vec!["rename", "2", "two", "--words", "here"],
                serde_json::json!({"id":"cli:tab:rename","method":"tab.rename","params":{"tab_id":"2","label":"two --words here"}}),
            ),
            (
                vec!["get", "2"],
                serde_json::json!({"id":"cli:tab:get","method":"tab.get","params":{"tab_id":"2"}}),
            ),
            (
                vec!["focus", "2"],
                serde_json::json!({"id":"cli:tab:focus","method":"tab.focus","params":{"tab_id":"2"}}),
            ),
            (
                vec!["close", "tab-2"],
                serde_json::json!({"id":"cli:tab:close","method":"tab.close","params":{"tab_id":"tab-2"}}),
            ),
            (
                vec!["floats-toggle"],
                serde_json::json!({"id":"cli:tab:floats_toggle","method":"tab.floats_toggle","params":{"mode":"toggle"}}),
            ),
            (
                vec!["floats-toggle", "--workspace", "2", "--off"],
                serde_json::json!({"id":"cli:tab:floats_toggle","method":"tab.floats_toggle","params":{"workspace_id":"2","mode":"off"}}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(parse(&args).unwrap()).unwrap(),
                expected,
                "{args:?}"
            );
        }
        assert!(
            matches!(parse(&["create", "--env", "=value"]), Err(ParseError::Usage(message)) if message == "env key must not be empty")
        );
    }

    #[test]
    fn tab_projection_clap_grammar_and_ordered_focus() {
        let value = serde_json::to_value(
            parse(&[
                "create",
                "--focus",
                "--no-focus",
                "--focus",
                "--cwd=relative",
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["params"],
            serde_json::json!({"cwd":"relative", "focus":true})
        );
        assert!(
            matches!(parse(&["get", "2", "--help"]), Err(ParseError::Clap(error)) if error.exit_code() == 0)
        );
        for args in [
            vec!["get"],
            vec!["get", "2", "extra"],
            vec!["create", "--cwd"],
            vec!["floats-toggle", "--on", "--off"],
        ] {
            assert!(matches!(parse(&args), Err(ParseError::Clap(error)) if error.exit_code() == 2));
        }
        command().debug_assert();
    }
}
