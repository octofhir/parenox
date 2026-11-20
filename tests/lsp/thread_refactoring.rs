use super::*;

const SOURCE: &str = "(ns demo)\n(defn build [xs] [\"🧪\" (assoc ; outer\n  (into {} (reverse xs)) ; map\n  :done true)])\n";
const OVERLAY: &str = "(ns demo)\n(defn build [xs] [\"🧪\" (assoc ; outer\n  (conj {} [:x xs]) ; map\n  :done true)])\n";
const TITLE: &str = "Thread first all";
const COMMAND: &str = "thread-first-all";

fn execute_command(
    id: usize,
    file: &str,
    command: &str,
    (line, character): (usize, usize),
) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":command,"arguments":[file,line,character]
    }})
}

fn code_actions(id: usize, file: &str, (line, character): (usize, usize), kind: &str) -> Value {
    let position = json!({"line":line,"character":character});
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
        "textDocument":{"uri":file},"range":{"start":position,"end":position},
        "context":{"diagnostics":[],"only":[kind]}
    }})
}

fn thread_action<'a>(response: &'a Value, title: &str) -> &'a Value {
    response["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["title"] == title)
        .unwrap_or_else(|| panic!("missing {title} action in {response}"))
}

fn expected_edit(text: &str) -> Value {
    edit((1, 23), (3, 13), text)
}

fn edit(
    (start_line, start_col): (usize, usize),
    (end_line, end_col): (usize, usize),
    text: &str,
) -> Value {
    json!([{
        "range":{"start":{"line":start_line,"character":start_col},"end":{"line":end_line,"character":end_col}},
        "newText":text
    }])
}

#[test]
fn thread_first_all_keeps_trivia_and_exposes_the_same_editor_command() {
    let fixture = Fixture::new("thread first all");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        code_actions(2, &file, (1, 23), "refactor"),
        code_actions(3, &file, (1, 23), "quickfix"),
        execute_command(4, &file, COMMAND, (1, 23)),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| {
        responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()
    };
    let commands = response(1)["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&json!(COMMAND)));
    assert_eq!(
        thread_action(response(2), TITLE),
        &json!({"title":TITLE,"kind":"refactor.rewrite","command":{
            "title":TITLE,"command":COMMAND,"arguments":[file,1,23]
        }})
    );
    assert_eq!(response(3)["result"], json!([]));
    assert_eq!(
        response(4)["result"]["changes"][&file],
        expected_edit(concat!(
            "(-> {}\n",
            "                           (into (reverse xs))\n",
            "                           (assoc ; outer\n",
            "  ; map\n",
            "  :done true))",
        ))
    );
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

#[test]
fn thread_first_all_editor_action_uses_the_live_version_and_waits_for_the_client() {
    let fixture = Fixture::new("thread first all overlay");
    let file = uri(&fixture.0.canonicalize().unwrap().join("src/demo.clj"));
    let probe = first_all_probe();
    let result = refactor_transcript(lsp_server(), &fixture.0, &probe);
    assert_eq!(
        result.unsaved_action["command"]["arguments"],
        json!([file, 1, 24])
    );
    let edits = expected_edit(concat!(
        "(-> {}\n",
        "                           (conj [:x xs])\n",
        "                           (assoc ; outer\n",
        "  ; map\n",
        "  :done true))",
    ));
    assert_eq!(
        result.unsaved.apply["documentChanges"],
        json!([{
            "textDocument":{"uri":file,"version":7},"edits":edits
        }])
    );
    assert_eq!(result.saved.reply["result"], result.saved.apply);
    assert_eq!(result.unsaved.reply["result"], result.unsaved.apply);
    assert_eq!(result.rejected.apply, result.unsaved.apply);
    assert_eq!(
        result.rejected.reply["error"],
        json!({"code":-32803,"message":"version conflict"})
    );
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_thread_first_all_with_reference() {
    let fixture = Fixture::new("thread first all parity");
    let probe = first_all_probe();
    compare_reference(&fixture.0, &probe);
}

fn first_all_probe() -> RefactorProbe<'static> {
    RefactorProbe {
        title: TITLE,
        standalone_command: None,
        source: SOURCE,
        overlay: OVERLAY,
        saved_position: (1, 23),
        unsaved_position: (1, 24),
    }
}

const LAST_ALL: RefactorProbe<'static> = RefactorProbe {
    title: "Thread last all",
    standalone_command: None,
    source: concat!(
        "(ns demo)\n",
        "(defn last-chain [xs] [\"🧪\" (remove nil? (filter :id (map identity xs)))])\n",
    ),
    overlay: concat!(
        "(ns demo)\n",
        "(defn last-chain [xs] [\"🧪\" (remove nil? ; predicate\n",
        "    (filter :ok (map str xs)) ; input\n",
        "    )])\n",
    ),
    saved_position: (1, 28),
    unsaved_position: (1, 29),
};

const LAST_ONCE: RefactorProbe<'static> = RefactorProbe {
    standalone_command: Some("thread-last"),
    saved_position: (1, 71),
    unsaved_position: (1, 28),
    ..LAST_ALL
};

const UNWIND_ALL: RefactorProbe<'static> = RefactorProbe {
    title: "Unwind whole thread",
    standalone_command: None,
    source: "(ns demo)\n(defn inverse [xs] [\"🧪\" (-> xs reverse (conj :last))])\n",
    overlay: concat!(
        "(ns demo)\n",
        "(defn inverse [xs] [\"🧪\" (->> xs (take 2 ; before close\n",
        "    ) vec)])\n",
    ),
    saved_position: (1, 25),
    unsaved_position: (1, 26),
};

const UNWIND_ONCE: RefactorProbe<'static> = RefactorProbe {
    title: "Unwind thread once",
    standalone_command: None,
    source: "(ns demo)\n(defn inverse [xs] [\"🧪\" (->> xs reverse (take 3) vec)])\n",
    overlay: concat!(
        "(ns demo)\n",
        "(defn inverse [xs] [\"🧪\" (->> xs ; input\n",
        "       (map identity) ; after map\n",
        "       (take 3))])\n",
    ),
    saved_position: (1, 25),
    unsaved_position: (2, 8),
};

#[test]
fn thread_last_rewrites_a_single_step_at_an_opener_or_the_adjacent_closer() {
    let fixture = Fixture::new("thread last once");
    let result = refactor_transcript(lsp_server(), &fixture.0, &LAST_ONCE);
    assert_rewrites(
        &result,
        "thread-last-all",
        edit(
            (1, 41),
            (1, 71),
            concat!(
                "(->> (map identity xs)\n",
                "                                              (filter :id))",
            ),
        ),
        edit(
            (1, 28),
            (3, 5),
            concat!(
                "(->> (filter :ok (map str xs))\n",
                "                                 (remove nil? ; predicate\n",
                "; input\n",
                "    ))",
            ),
        ),
    );
}

#[test]
fn thread_last_guards_cursor_positions_and_preserves_reader_trivia() {
    let fixture = Fixture::new("thread last cursor and trivia");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let source = concat!(
        "(ns demo)\n",
        "(remove nil? (filter :id (map identity xs)))\n",
        "(vec ; before input\n",
        "  [1 2])\n",
        "(remove nil? (list 1 2) #_:skip)\n",
        "(vector #_:ignore)\n",
        "(list (vec []) (reverse []))\n",
    );
    fs::write(&path, source).unwrap();
    let positions = [(2, 0), (4, 0), (1, 1), (1, 41), (5, 0), (6, 14)];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    requests.extend(
        positions
            .into_iter()
            .enumerate()
            .map(|(index, position)| execute_command(index + 2, &file, "thread-last", position)),
    );
    requests.extend([
        json!({"jsonrpc":"2.0","id":8,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let rewrite = |id| &response(id)["result"]["changes"][&file][0]["newText"];
    assert_eq!(rewrite(2), "(->> [1 2]\n     (vec ; before input\n))");
    assert_eq!(rewrite(3), "(->> (list 1 2)\n     (remove nil? #_:skip))");
    for id in 4..8 {
        assert_eq!(
            response(id)["error"],
            json!({"code":-32600,"message":"Nothing to change."})
        );
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_thread_last_once_with_reference() {
    let fixture = Fixture::new("thread last once parity");
    compare_reference(&fixture.0, &LAST_ONCE);
}

#[test]
fn thread_last_all_threads_the_last_argument_and_preserves_comments() {
    let fixture = Fixture::new("thread last all");
    let result = refactor_transcript(lsp_server(), &fixture.0, &LAST_ALL);
    assert_rewrites(
        &result,
        "thread-last-all",
        edit(
            (1, 28),
            (1, 72),
            concat!(
                "(->> xs\n",
                "                                 (map identity)\n",
                "                                 (filter :id)\n",
                "                                 (remove nil?))",
            ),
        ),
        edit(
            (1, 28),
            (3, 5),
            concat!(
                "(->> xs\n",
                "                                 (map str)\n",
                "                                 (filter :ok)\n",
                "                                 (remove nil? ; predicate\n",
                "; input\n",
                "    ))",
            ),
        ),
    );
}

#[test]
fn unwind_all_rebuilds_thread_first_and_last_chains() {
    let fixture = Fixture::new("unwind all");
    let result = refactor_transcript(lsp_server(), &fixture.0, &UNWIND_ALL);
    assert_rewrites(
        &result,
        "unwind-all",
        edit((1, 25), (1, 53), "(conj (reverse xs) :last)"),
        edit((1, 25), (2, 10), "(vec (take 2 xs ; before close\n    ))"),
    );
}

#[test]
fn unwind_thread_collapses_one_step_from_the_cursor_and_preserves_comments() {
    let fixture = Fixture::new("unwind once");
    let result = refactor_transcript(lsp_server(), &fixture.0, &UNWIND_ONCE);
    assert_rewrites(
        &result,
        "unwind-thread",
        edit((1, 25), (1, 54), "(->> (reverse xs) (take 3) vec)"),
        edit(
            (1, 25),
            (3, 16),
            "(->> ; input\n       (map identity xs) ; after map\n       (take 3))",
        ),
    );
}

fn assert_rewrites(
    result: &RefactorTranscript,
    action_command: &str,
    saved: Value,
    unsaved: Value,
) {
    assert_eq!(result.saved_action["command"]["command"], action_command);
    assert_eq!(result.unsaved_action["command"]["command"], action_command);
    let saved_document = &result.saved.apply["documentChanges"][0];
    let unsaved_document = &result.unsaved.apply["documentChanges"][0];
    assert_eq!(saved_document["edits"], saved);
    assert_eq!(saved_document["textDocument"]["version"], 1);
    assert_eq!(unsaved_document["edits"], unsaved);
    assert_eq!(unsaved_document["textDocument"]["version"], 7);
    assert_eq!(result.saved.reply["result"], result.saved.apply);
    assert_eq!(result.unsaved.reply["result"], result.unsaved.apply);
    assert_eq!(result.rejected.apply, result.unsaved.apply);
    assert_eq!(
        result.rejected.reply["error"],
        json!({"code":-32803,"message":"version conflict"})
    );
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_thread_last_all_with_reference() {
    let fixture = Fixture::new("thread last all parity");
    compare_reference(&fixture.0, &LAST_ALL);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_unwind_all_with_reference() {
    let fixture = Fixture::new("unwind all parity");
    compare_reference(&fixture.0, &UNWIND_ALL);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_unwind_thread_with_reference() {
    let fixture = Fixture::new("unwind once parity");
    compare_reference(&fixture.0, &UNWIND_ONCE);
}

#[test]
fn unwind_preserves_outer_comments_and_refuses_noncore_or_incomplete_threads() {
    let fixture = Fixture::new("unwind guard");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let source = format!(
        "{}(defn short-circuit [x] (some-> x inc))\n(defn no-steps [x] (-> x))\n(defn local [-> x] (-> x inc))\n",
        UNWIND_ONCE.overlay
    );
    fs::write(&path, &source).unwrap();
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        execute_command(2, &file, "unwind-all", (1, 25)),
    ];
    let positions = [(4, 24), (5, 19), (6, 19)];
    for (index, position) in positions.into_iter().enumerate() {
        let id = 3 + index * 3;
        requests.extend([
            code_actions(id, &file, position, "refactor"),
            execute_command(id + 1, &file, "unwind-thread", position),
            execute_command(id + 2, &file, "unwind-all", position),
        ]);
    }
    requests.extend([
        json!({"jsonrpc":"2.0","id":12,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let rewrites = &response(2)["result"]["changes"][&file];
    let rewritten = rewrites[0]["newText"].as_str().unwrap();
    for comment in ["; input", "; after map"] {
        assert_eq!(rewritten.matches(comment).count(), 1, "{rewritten}");
    }
    for index in 0..positions.len() {
        let id = 3 + index * 3;
        let actions = response(id)["result"].as_array().unwrap();
        assert!(actions.iter().all(|action| !matches!(
            action["title"].as_str(),
            Some("Unwind thread once" | "Unwind whole thread")
        )));
        for command_id in [id + 1, id + 2] {
            assert_eq!(
                response(command_id)["error"],
                json!({"code":-32600,"message":"Nothing to change."})
            );
        }
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn threading_keeps_adjacent_discard_tokens_separate_from_symbols() {
    let fixture = Fixture::new("thread discard adjacency");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, "(ns demo)\n(vec(xs)#_:skip)\n(-> xs (vec)#_:skip)\n").unwrap();
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        execute_command(2, &file, "thread-first-all", (1, 0)),
        execute_command(3, &file, "unwind-all", (2, 0)),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let rewrite = |id| {
        replies.iter().find(|message| message["id"] == id).unwrap()["result"]["changes"][&file][0]["newText"].as_str().unwrap()
    };
    assert_eq!(rewrite(2), "(-> (xs)\n    (vec #_:skip))");
    assert_eq!(rewrite(3), "(vec xs #_:skip)");
}

struct RefactorProbe<'a> {
    title: &'a str,
    standalone_command: Option<&'a str>,
    source: &'a str,
    overlay: &'a str,
    saved_position: (usize, usize),
    unsaved_position: (usize, usize),
}

struct CommandExchange {
    apply: Value,
    reply: Value,
}

struct RefactorTranscript {
    saved_action: Value,
    unsaved_action: Value,
    saved: CommandExchange,
    rejected: CommandExchange,
    unsaved: CommandExchange,
}

fn compare_reference(root: &Path, probe: &RefactorProbe<'_>) {
    let reference =
        std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let child = Command::new(reference)
        .arg("listen")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let candidate = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    let reference = refactor_transcript(child, root, probe);
    let candidate = refactor_transcript(lsp_server_with(Path::new(&candidate)), root, probe);
    assert_eq!(reference.saved_action, candidate.saved_action);
    assert_eq!(reference.unsaved_action, candidate.unsaved_action);
    assert_eq!(reference.saved.reply["result"], reference.saved.apply);
    assert_eq!(candidate.saved.reply["result"], candidate.saved.apply);

    // The reference numbers opened documents from 0 until it receives didChange.
    let mut saved = reference.saved.apply;
    assert_eq!(saved["documentChanges"][0]["textDocument"]["version"], 0);
    assert_eq!(
        candidate.saved.apply["documentChanges"][0]["textDocument"]["version"],
        1
    );
    saved["documentChanges"][0]["textDocument"]["version"] = json!(1);
    assert_eq!(saved, candidate.saved.apply);
    assert_eq!(
        reference.unsaved.apply["documentChanges"][0]["textDocument"]["version"],
        7
    );
    assert_eq!(reference.unsaved.apply, candidate.unsaved.apply);
    assert_eq!(reference.unsaved.reply["result"], reference.unsaved.apply);
    assert_eq!(candidate.unsaved.reply["result"], candidate.unsaved.apply);
    assert_eq!(reference.rejected.apply, reference.unsaved.apply);
    assert_eq!(candidate.rejected.apply, candidate.unsaved.apply);
    assert_eq!(reference.rejected.reply["result"], reference.rejected.apply);
    assert_eq!(
        candidate.rejected.reply["error"],
        json!({"code":-32803,"message":"version conflict"})
    );
}

fn refactor_transcript(
    mut child: Child,
    root: &Path,
    probe: &RefactorProbe<'_>,
) -> RefactorTranscript {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, probe.source).unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message: Value, accepted: Option<bool>| {
        send_message(&mut input, &message);
        if message["id"].is_null() {
            return (Value::Null, None);
        }
        let mut apply = None;
        loop {
            let reply = read_message(&mut output).expect("LSP message");
            if reply["method"].is_null() && reply["id"] == message["id"] {
                return (reply, apply);
            }
            if !reply["id"].is_null() && !reply["method"].is_null() {
                let result = if reply["method"] == "workspace/applyEdit" {
                    assert!(apply.replace(reply["params"]["edit"].clone()).is_none());
                    match accepted.expect("unexpected workspace edit") {
                        true => json!({"applied":true}),
                        false => json!({"applied":false,"failureReason":"version conflict"}),
                    }
                } else {
                    Value::Null
                };
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":result}),
                );
            }
        }
    };
    let (initialize, _) = exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&root),"capabilities":{"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true}}}
        }}),
        None,
    );
    if let Some(command) = probe.standalone_command {
        let commands = initialize["result"]["capabilities"]["executeCommandProvider"]["commands"]
            .as_array()
            .unwrap();
        assert!(commands.contains(&json!(command)));
    }
    exchange(
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        None,
    );
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":probe.source}
        }}),
        None,
    );
    let saved_action = probe_action(
        &exchange(
            code_actions(2, &file, probe.saved_position, "refactor.rewrite"),
            None,
        )
        .0,
        probe,
    )
    .clone();
    let saved = execute_refactoring(
        &mut exchange,
        3,
        &saved_action,
        probe.standalone_command,
        true,
    );
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":probe.overlay}]
        }}),
        None,
    );
    let unsaved_action = probe_action(
        &exchange(
            code_actions(4, &file, probe.unsaved_position, "refactor.rewrite"),
            None,
        )
        .0,
        probe,
    )
    .clone();
    let rejected = execute_refactoring(
        &mut exchange,
        5,
        &unsaved_action,
        probe.standalone_command,
        false,
    );
    let unsaved = execute_refactoring(
        &mut exchange,
        6,
        &unsaved_action,
        probe.standalone_command,
        true,
    );
    exchange(json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), probe.source);
    RefactorTranscript {
        saved_action,
        unsaved_action,
        saved,
        rejected,
        unsaved,
    }
}

fn probe_action<'a>(response: &'a Value, probe: &RefactorProbe<'_>) -> &'a Value {
    if let Some(command) = probe.standalone_command {
        assert!(
            response["result"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["command"] != command && item["command"]["command"] != command)
        );
    }
    thread_action(response, probe.title)
}

fn execute_refactoring(
    exchange: &mut impl FnMut(Value, Option<bool>) -> (Value, Option<Value>),
    id: usize,
    action: &Value,
    standalone: Option<&str>,
    accepted: bool,
) -> CommandExchange {
    let command =
        standalone.map_or_else(|| action["command"]["command"].clone(), |name| json!(name));
    let (reply, apply) = exchange(
        json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
            "command":command,"arguments":action["command"]["arguments"]
        }}),
        Some(accepted),
    );
    CommandExchange {
        apply: apply.unwrap_or_else(|| panic!("missing workspace/applyEdit for {reply}")),
        reply,
    }
}
