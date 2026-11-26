#!/usr/bin/env bash
# Break-it checks: each rule is removed, one at a time, in a scratch copy of the source, and the
# test that claims the rule must then fail. The source folder is never changed.
#
# Usage: checks/break-it.sh [SOURCE_DIR]      (default: this repo, as its working tree stands)
# Output: one line per check. "fails (good)" is the wanted result; anything else is a finding.
# Exit: 0 when every check failed its test as it should, 1 otherwise.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
src=${1:-$here/..}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -C "$src" --exclude=./target --exclude=./.git -cf - . | tar -C "$work" -xf -
cd "$work" || exit 1
mkdir .pristine && cp -r src .pristine/

echo "source: $src"
echo "building and testing the unchanged copy first..."
cargo build -q --tests 2>/dev/null || { echo "the unchanged copy does not build"; exit 1; }
cargo test -q >/dev/null 2>&1 || { echo "the unchanged copy fails its tests"; exit 1; }

status=0
# check NAME FILE TEST_FILE TEST_NAME PERL_SUBSTITUTION
check() {
    local name=$1 file=$2 tfile=$3 tname=$4 expr=$5
    cp -r .pristine/src/. src/
    perl -0pi -e "$expr" "src/$file"
    if cmp -s ".pristine/src/$file" "src/$file"; then
        echo "NOT APPLIED   $name"; status=1; return
    fi
    if ! cargo build -q --tests 2>/dev/null; then
        echo "NO BUILD      $name"; status=1; return
    fi
    local target=(--test "$tfile")
    [ "$tfile" = lib ] && target=(--lib)
    [ "$tfile" = bin ] && target=(--bin focus-board)
    if cargo test -q "${target[@]}" "$tname" >/dev/null 2>&1; then
        echo "STILL PASSES  $name   ($tfile::$tname)"; status=1
    else
        echo "fails (good)  $name   ($tfile::$tname)"
    fi
}

# The store: versions, ownership, transactions, history kept by triggers.
check "no version check"            store.rs store stale_version_refused 's/&& read != current/&& false/'
check "cells may be updated"        store.rs store update_and_delete_refused 's/cells are never changed\x27\); END;/cells are never changed\x27) WHERE 0; END;/'
check "REPLACE may overwrite cells" store.rs store update_and_delete_refused 's/WHEN EXISTS \(SELECT 1 FROM cells WHERE id = NEW\.id\)\n\s*OR NEW\.version/WHEN 0 AND NEW.version/'
check "REPLACE may overwrite items" store.rs store update_and_delete_refused 's/WHEN EXISTS \(SELECT 1 FROM items WHERE id = NEW\.id\)/WHEN 0/'
check "REPLACE may take a writer"   store.rs store update_and_delete_refused 's/WHEN EXISTS \(SELECT 1 FROM writers WHERE id = NEW\.id\n\s*OR \(session_id = NEW\.session_id AND agent_id = NEW\.agent_id\)\)/WHEN 0/'
check "agent folds into session"    store.rs store fork_and_parent_own_cells 's/find_writer\(tx, &who\.session_id, &who\.agent_id\)\?/find_writer(tx, &who.session_id, "")?/'
check "deferred transactions"       store.rs store concurrent_writers_both_land 's/TransactionBehavior::Immediate/TransactionBehavior::Deferred/g'
check "answer open to sessions"     store.rs store session_cannot_write_the_owners_column 's/Field::Answer if !is_owner =>/Field::Answer if false =>/'
check "old timeline node filter"    store.rs store fork_in_tree_and_timeline_order 's/a\.id IN branch OR w\.id IN branch/w.id IN branch/'

# Identity and place.
check "worktree is its own project" identity.rs store projects_and_worktrees_stay_apart 's/project: shown\(&project\)/project: shown(&top)/'
check "home repo is a project"      identity.rs store projects_and_worktrees_stay_apart 's/Some\(top\) if Some\(&top\) != home\.as_ref\(\) =>/Some(top) =>/'
check "shell session id counts"     identity.rs command unidentified_command_write_refused 's/let session_id = get\(SESSION_VAR\)\.unwrap_or_default\(\);/let session_id = get(SESSION_VAR).or_else(|| get("CLAUDE_CODE_SESSION_ID")).unwrap_or_default();/'

# The command and the hook.
check "answer accepted"             main.rs command cli_refuses_answer 's/Cmd::Answer \{ \.\. \} => Err\(format!\(/Cmd::Answer { .. } if true => Ok(()),\n        Cmd::Answer { .. } => Err(format!(/'
check "list ignores the project"    main.rs command command_round_trip 's/if !everywhere \{/if false && !everywhere {/'
check "hook rewrites everything"    hook.rs command hook_rewrites_only_board_calls 's/if !runs_board\(command\) \{/if false && !runs_board(command) {/'
check "hook drops the agent id"     hook.rs command hook_rewrites_only_board_calls 's/\{AGENT_VAR\}=\x27\{agent\}\x27/{AGENT_VAR}=\x27\x27/'

# The server's guards and feed.
check "any Host"                    server.rs server server_guards 's/if !hosts\.iter\(\)\.any\(\|h\| h == host\) \{/if false && !hosts.iter().any(|h| h == host) {/'
check "Host checked on writes only" server.rs server server_guards 's/if !hosts\.iter\(\)\.any\(\|h\| h == host\) \{/if method != Method::GET && !hosts.iter().any(|h| h == host) {/'
check "any Origin"                  server.rs server server_guards 's#if origin != format!\("http://\{host\}"\) \{#if false && origin != format!("http://{host}") {#'
check "no custom header needed"     server.rs server server_guards 's/if text\(header::HeaderName::from_static\(WRITE_HEADER\)\) != Some\("1"\) \{/if false {/'
check "change feed never moves"     server.rs server page_sees_writes_made_past_it 's/"changed": last > after/"changed": (false && last > after)/'
check "stale answer accepted"       server.rs server page_answers_and_reads 's/WriteError::Stale \{ \.\. \} => StatusCode::CONFLICT,/WriteError::Stale { .. } => StatusCode::OK,/'


# Added after review 02 (R1, R2, R6, R8, R10, R11, G8 and its notes).
check "hand-set identity allowed"    hook.rs command hook_denies_a_hand_set_identity 's/if sets_identity\(command\) \{/if false && sets_identity(command) {/'
check "refusal invites hand-setting" identity.rs command unidentified_command_write_refused 's/Never set FOCUS_BOARD_\* yourself/Set FOCUS_BOARD_SESSION yourself/'
check "hook takes any id"            hook.rs command hook_rewrites_only_board_calls 's/!\[session, agent, agent_type\]/false && ![session, agent, agent_type]/'
check "board may be framed"          server.rs server page_cannot_be_framed 's/\.layer\(middleware::map_response\(never_framed\)\)//'
check "shared folder name mixes"     main.rs command a_shared_folder_name_does_not_mix_projects 's/\[one\] => Ok\(one\.to_string\(\)\),/[one, ..] => Ok(one.to_string()),/'
check "repeated letter kept"         store.rs store a_repeated_option_letter_is_refused 's/\.find\(\|\(k, _\)\| !letters\.insert\(k\.as_str\(\)\)\)/.find(|(k, _)| false && !letters.insert(k.as_str()))/'
check "newer layout opened"          store.rs store a_newer_layout_is_refused 's/if layout > LAYOUT \{/if false && layout > LAYOUT {/'
check "anyone writes the question"   store.rs store only_the_author_writes_the_question 's/Field::Question \| Field::Options \| Field::Recommended if writer != author =>/Field::Question | Field::Options | Field::Recommended if false =>/'
check "timeline never cut"           server.rs server timeline_is_cut_and_says_so 's/\.timeline\(&filter, after, Some\(limit\)\)\?/.timeline(&filter, after, None)?/'
check "closed counts as active"      store.rs store timeline_cut_and_active_filter 's/"active" => i\.status != "closed",/"active" => true,/'
check "cells may be deleted"         store.rs store update_and_delete_refused 's/cells are never deleted\x27\); END;/cells are never deleted\x27) WHERE 0; END;/'
check "status set by a non-author"   store.rs command command_round_trip 's/Field::Status if !is_owner && writer != author =>/Field::Status if false =>/'

# Added with the item context (owner, 2026-11-21).
check "item context not recorded"    store.rs store item_context_is_recorded_at_raise_time 's/place\.branch\n(\s*)\]/""\n$1]/'
check "branch not detected"          identity.rs store projects_and_worktrees_stay_apart 's/branch: branch_of\(&dir\),/branch: String::new(),/'
check "unversioned board opened"     store.rs store an_unversioned_board_is_refused 's/if layout == 0 && has_items \{/if false && layout == 0 && has_items {/'

# Added with the copied-session parent (review R3, the owner's /branch and /fork checks).
check "copy ignores its parent"      store.rs store a_copied_session_files_under_its_parent 's/if parent\.is_empty\(\) \|\| parent == &who\.session_id \{/if true {/'
check "node filter one level"        store.rs store a_copied_session_files_under_its_parent 's/JOIN branch b ON w2\.parent = b\.id\)/JOIN branch b ON w2.parent = b.id AND 0)/g'
check "parent is the chain's root"   identity.rs store fork_parent_reads_the_transcript 's/last_request\.or\(forked_from\)/forked_from.or(last_request)/'
check "command skips the lookup"     main.rs command a_copied_session_writes_under_its_parent 's/who\.parent_session = parent;/let _ = parent;/'
check "transcript hand-set allowed"  hook.rs command hook_denies_a_hand_set_identity 's/\[SESSION_VAR, AGENT_VAR, AGENT_TYPE_VAR, TRANSCRIPT_VAR\]/[SESSION_VAR, AGENT_VAR, AGENT_TYPE_VAR]/'
check "item filed at its writer's place" store.rs store an_item_is_filed_where_it_was_raised 's/AND \(\?3 IS NULL OR i\.workspace = \?3\)/AND (?3 IS NULL OR a.workspace = ?3)/; s/AND \(\?2 IS NULL OR i\.workspace = \?2\)/AND (?2 IS NULL OR a.workspace = ?2)/'
check "Windows board name missed"    hook.rs lib runs_board_finds_the_command_word_only 's/lower == "focus-board" \|\| lower == "focus-board\.exe"/lower == "focus-board"/'
check "drive path taken as unsafe"   identity.rs store windows_paths_are_understood 's/\(path\.starts_with\(\x27\/\x27\) \|\| is_drive_path\(path\)\)/path.starts_with(\x27\/\x27)/'
check "\\\\?\\ prefix kept"          identity.rs store windows_paths_are_understood 's/Some\(rest\) if is_drive_path\(rest\) => rest\.to_string\(\),/Some(rest) if false \&\& is_drive_path(rest) => rest.to_string(),/'
check "a bad line ends the scan"     identity.rs store fork_parent_reads_the_transcript 's/let line = String::from_utf8_lossy\(&bytes\);/let Ok(line) = String::from_utf8(bytes) else { break };/'
check "Windows path taken as a name" main.rs bin paths_of_any_os 's/arg == "\." \|\| arg\.contains\(\[\x27\/\x27, \x27\\\\\x27\]\) \|\| is_drive_path\(arg\)/arg == "." || arg.contains(\x27\/\x27)/'
check "folders split on / only"      main.rs bin paths_of_any_os 's/\.rsplit\(\[\x27\/\x27, \x27\\\\\x27\]\)/.rsplit(\x27\/\x27)/'
check "where ignores the data folder" lib.rs command where_shows_the_board 's/base\.data_dir\(\)\.join/base.home_dir().join/'
check "the label beats the title"    identity.rs store session_names_read_from_the_transcript 's/self\.title\.clone\(\)\.or_else\(\|\| self\.label\.clone\(\)\)/self.label.clone().or_else(|| self.title.clone())/'
check "a later label hides a title"  identity.rs names the_watch_follows_a_rename 's/title: later\.title\.or\(self\.title\),/title: later.title,/'
check "half lines are read"          names.rs names the_watch_follows_a_rename 's/let whole = bytes\.iter\(\)\.rposition\(\|&b\| b == b\x27\\n\x27\)\.map_or\(0, \|i\| i \+ 1\);/let whole = bytes.len();/'
check "renames are not watched"      server.rs server a_rename_reaches_the_board_without_a_write 's/tokio::spawn\(watch_renames\(app\.clone\(\), watcher\)\);/let _ = watcher;/'
check "the first title wins"         identity.rs store session_names_read_from_the_transcript 's/title = text\("customTitle"\)\.filter\(\|t\| !t\.is_empty\(\)\)\.or\(title\)/title = title.or(text("customTitle").filter(|t| !t.is_empty()))/'
check "sessions show as hashes"      store.rs store a_session_name_follows_a_rename 's/\(true, true\) => made_up_name\(session_id\),/(true, true) => String::new(),/'
check "a rename is not followed"     store.rs store a_session_name_follows_a_rename 's/if who\.agent_id\.is_empty\(\) && !who\.name\.is_empty\(\) \{/if false {/'
check "old writers stay unnamed"     store.rs store a_session_name_follows_a_rename 's/fill_names\(&conn\)\?;//'
check "names not read on write"      main.rs command a_session_shows_its_rename_title 's/who\.name = name;/let _ = name;/'
check "wait ignores the answer"      main.rs command wait_ends_when_the_owner_answers 's/if answered > after\.unwrap_or\(0\) \|\| closed \{/if closed {/'
check "WSL shares the Linux port"    lib.rs command each_platform_has_its_own_port 's/"linux" if kernel_release/"linux" if false \&\& kernel_release/'
check "Windows shares a port"        lib.rs command each_platform_has_its_own_port 's/"windows" => 8812/"windows" => 8810/'
check "a wait reopens the board"     main.rs command a_wait_survives_a_layout_upgrade_while_it_waits 's/return show_from\(&store, item, false\);/return show(item, false);/'
check "own rewrite denied when re-sent" hook.rs command the_hooks_own_rewrite_passes_when_sent_again 's/&& !sets_identity\(rest\)/\&\& false \&\& !sets_identity(rest)/'
check "a title beats the owner's rename" names.rs names the_registry_names_running_sessions 's/from\("user"\)\n(\s*)\.or_else\(\|\| names\.title\.clone\(\)\)/names.title.clone()\n$1.or_else(|| from("user"))/'
check "derived names are used"       names.rs names the_registry_names_running_sessions 's/\.or_else\(\|\| names\.label\.clone\(\)\)\n\}/.or_else(|| names.label.clone())\n        .or_else(|| from("derived"))\n}/'
check "the older registry name wins" names.rs names the_registry_names_running_sessions 's/Some\(known\) if known\.since >= since => \{\}/Some(known) if known.since <= since => {}/'
check "a write ignores the registry" main.rs command a_write_takes_the_registry_name 's/best_name\(registered\.get\(&who\.session_id\)\.or\(copilot\.as_ref\(\)\), &names\)/best_name(copilot.as_ref(), \&names)/'
check "new cells are not indexed"    store.rs store full_text_search_finds_any_cell_text 's/cells_indexed AFTER INSERT ON cells BEGIN/cells_indexed AFTER DELETE ON cells BEGIN/'
check "old cells are not indexed"    store.rs store an_older_board_is_brought_to_layout_2 's/FROM cells c;/FROM cells c WHERE 0;/'
check "odd characters reach the index" store.rs store full_text_search_finds_any_cell_text 's/format!\("\\"\{\}\\"", word/format!("{}", word/'
check "a word's start does not match" store.rs store full_text_search_finds_any_cell_text 's/quoted\(w\) \+ "\*"/quoted(w)/'
check "any one word is enough"       store.rs store full_text_search_finds_any_cell_text 's/so_far\.intersection\(&items\)/so_far.union(\&items)/'
check "words must share one cell"    store.rs store every_word_may_sit_in_a_different_cell 's/(if terms\.is_empty\(\) \{\s*return Ok\(Vec::new\(\)\);\s*\})/$1\n        return self.conn.prepare("SELECT DISTINCT item FROM cell_text WHERE cell_text MATCH ?1")?.query_map(params![terms.join(" ")], |r| r.get(0))?.collect();/'
check "a reference matches as a start" store.rs store search_finds_an_item_by_its_reference 's/if is_ref\(w\) \{/if false \&\& is_ref(w) {/'
check "new items are not indexed"    store.rs store search_finds_an_item_by_its_reference 's/items_indexed AFTER INSERT ON items BEGIN/items_indexed AFTER DELETE ON items BEGIN/'
check "old items are not indexed"    store.rs store a_layout_2_board_gains_the_item_column 's/FROM items i;/FROM items i WHERE 0;/'
check "a press reaches a session twice" store.rs store a_review_request_reaches_each_session_once 's/if store\.has_seen_review\(request\.id, writer\)\.ok\(\)\? \{/if false {/; s/Ok\(added == 1\)/Ok(true)/'
check "the note goes to the parent"  store.rs store a_review_request_reaches_each_session_once 's/(let writer: WriterId = conn\s*\.query_row\(\s*"SELECT id FROM writers WHERE kind = \x27session\x27 AND session_id = \?1) AND agent_id = \?2/$1 AND ?2 = ?2/'
check "a closed item is in the note" store.rs store a_review_request_reaches_each_session_once 's/i\.author == writer && i\.status != "closed"/i.author == writer/'
check "a newer layout gets a note"   store.rs store no_note_from_a_missing_board_or_another_layout 's/if layout != LAYOUT \{/if layout < LAYOUT {/'
check "the hook creates a board"     store.rs store no_note_from_a_missing_board_or_another_layout 's/OpenFlags::SQLITE_OPEN_READ_WRITE \| OpenFlags::SQLITE_OPEN_NO_MUTEX/OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX/'
check "review requests can be deleted" store.rs store review_requests_and_seen_marks_are_never_changed 's/BEGIN SELECT RAISE\(ABORT, \x27review requests are never deleted\x27\); END;/BEGIN SELECT 1; END;/'
check "seen marks can be changed"    store.rs store review_requests_and_seen_marks_are_never_changed 's/BEGIN SELECT RAISE\(ABORT, \x27seen records are never changed\x27\); END;/BEGIN SELECT 1; END;/'
check "layout 4 is not applied"      store.rs store a_layout_3_board_gains_review_requests 's/if layout < 4 \{/if layout < 0 {/'
check "a denial carries the note"    hook.rs lib the_review_note_is_context_only_and_never_on_a_denial 's/if denied \{/if false {/'
check "the hook passes no note"      main.rs command the_hook_passes_the_review_note_on_once 's/hook::with_note\(hook::pre_tool_use\(&input\), note\)/hook::pre_tool_use(\&input)/'
check "wait --mine ignores a press"  main.rs command wait_mine_ends_on_each_of_its_reasons 's/if let Some\(request\) = store\.latest_review\(\)\.map_err\(err\)\?/if let Some(request) = None::<focus_board::store::ReviewRequest>/'
check "wait --mine ignores news"     main.rs command wait_mine_ends_on_each_of_its_reasons 's/if !news\.is_empty\(\) \{/if false {/'
check "wait --mine waits, none open" main.rs command wait_mine_ends_on_each_of_its_reasons 's/if open_items\.is_empty\(\) \{/if false {/'
check "wait --mine has no limit"     main.rs command wait_mine_ends_on_each_of_its_reasons 's/if started\.elapsed\(\) >= limit \{/if false {/'
check "wait --mine outlives its session" main.rs command wait_mine_ends_on_each_of_its_reasons 's/if !alive\(pid\) \{/if false {/'
check "--note-after still notes"    main.rs command copilot_gets_the_review_note_after_the_command 's/if note_after \{/if false {/'
check "post-tool-use gives no note" main.rs command copilot_gets_the_review_note_after_the_command 's/println!\("\{\}", hook::post_tool_note\(&note\)\);/let _ = note;/'
check "copilot names are ignored"   names.rs names copilot_sessions_have_their_names 's/if !is_plain_id\(session_id\) \|\| session_id\.is_empty\(\) \{/if true {/'
check "a session id leaves the folder" names.rs names copilot_sessions_have_their_names 's/if !is_plain_id\(session_id\) \|\| session_id\.is_empty\(\) \{/if session_id.is_empty() {/'
check "a copilot write ignores its name" main.rs command a_copilot_session_writes_under_its_copilot_name 's/\.and_then\(\|dir\| copilot_name\(&dir, &who\.session_id\)\)/.and_then(|_| None::<focus_board::names::Registered>)/'
check "copilot sessions get no mark" names.rs names a_sessions_tool_comes_from_its_files 's/if copilot\.is_some_and\(\|dir\| dir\.join\(session_id\)\.is_dir\(\)\) \{/if false {/'
check "a tool id leaves the folder" names.rs names a_sessions_tool_comes_from_its_files 's/if session_id\.is_empty\(\) \|\| !is_plain_id\(session_id\) \{/if session_id.is_empty() {/'
check "the tree drops the tool"     server.rs server the_tree_tells_each_sessions_tool 's/"tool": tools/"tool_": tools/'
check "a late wait misses earlier answers" main.rs command wait_mine_reports_an_answer_given_while_no_wait_ran 's/if !missed\.is_empty\(\) \{/if false {/'
check "an acted-on answer is news again" main.rs command wait_mine_reports_an_answer_given_while_no_wait_ran 's/if answer\.is_some_and\(\|a\| a > mine\) \{/if answer.is_some() {/'
check "a copilot session goes unnoticed" main.rs command wait_mine_ends_when_its_copilot_process_ends 's/"copilot" \| "copilot\.exe" \| //'
check "opencode names are ignored"  names.rs names opencode_sessions_have_their_names_and_mark 's/let title = opencode_title\(db, session_id\)\?;/let title: String = None::<String>?;/'
check "opencode placeholders are names" names.rs names opencode_sessions_have_their_names_and_mark 's/if title\.starts_with\("New session - "\)/if false/'
check "opencode sessions get no mark" names.rs names opencode_sessions_have_their_names_and_mark 's/if opencode\.is_some_and\(\|db\| opencode_title\(db, session_id\)\.is_some\(\)\) \{/if false {/'
check "an opencode write ignores its title" main.rs command an_opencode_session_writes_under_its_title 's/\.or_else\(\|\| opencode_db\(\)\.and_then\(\|db\| opencode_name\(&db, &who\.session_id\)\)\)//'
check "opencode 2 titles are not read" names.rs names opencode_sessions_have_their_names_and_mark 's/\["session_v2", "session"\]/["session"]/'
check "a skill block reaches every tool" skill.rs command each_tool_gets_its_own_skill 's/keep = names\.split_whitespace\(\)\.any\(\|name\| name == tool\);/keep = true;/'
check "a block's end drops the rest" skill.rs command each_tool_gets_its_own_skill 's/\} else if line == "<!-- end -->" \{\n(\s*)keep = true;/} else if line == "<!-- end -->" {\n$1keep = false;/'
check "new items are not told"      server.rs server the_owner_is_told_of_new_items_and_unseen_presses 's/\.filter\(\|i\| i\.id > after_item\)/.filter(|i| i.id > after_item \&\& false)/'
check "an unseen press is not told" server.rs server the_owner_is_told_of_new_items_and_unseen_presses 's/if !unseen\.is_empty\(\) \{/if false {/'
check "old items are news"          server.rs server the_owner_is_told_of_new_items_and_unseen_presses 's/Ok\(\(items\.iter\(\)\.map\(\|i\| i\.id\)\.max\(\)\.unwrap_or\(0\), press\)\)/Ok((0, press))/'
check "a wait is not announced"     live.rs command a_wait_announces_itself_while_it_runs 's/std::fs::write\(&path, body\.to_string\(\)\)\.ok\(\)\?;/None::<()>?;/'
check "an ended wait still counts"  live.rs live liveness_comes_from_live_processes_only 's/if !process_alive\(pid\) \{/if false {/'
check "an ended claude session runs" live.rs live liveness_comes_from_live_processes_only 's/pid > 0 \&\& process_alive\(pid\)/pid > 0/'
check "the tree drops liveness"     server.rs server the_tree_tells_whether_each_session_lives 's/"live": if waiting/"live_": if waiting/'
check "the feed misses reviews"      server.rs server the_review_button_asks_and_shows_who_has_seen_it 's/"reviews": store\.review_mark\(\)\?,/"reviews": 0,/'
check "unsafe transcript passed on"  hook.rs command a_copied_session_writes_under_its_parent 's/if is_safe_path\(transcript\) \{/if true || is_safe_path(transcript) {/'

cp -r .pristine/src/. src/
exit $status
