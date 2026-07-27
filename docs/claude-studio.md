# Claude Studio

`ALT+CTRL+U` — one window that holds every Claude Code conversation, the
way an editor holds every open file.

It is not a new application. It is **one kitty window running one tmux
session** on its own socket (`tmux -L claude-studio`, started with
`-f /dev/null`), so it can never inherit or restyle the real tmux server
you use for work — no plugins, no resurrect, no theme collision.

```
┌─ tab bar ────────────────────────────────────────────────────────┐
│ 󰚩 Claude Studio  0 sessions │ 1 fix-the-parser ✕ │ 2  api ✕   [|] [-] │
├──────────────────────────────────────────────────────────────────┤
│  the selected tab's window — one pane, or several                │
└──────────────────────────────────────────────────────────────────┘
```

## Tab 0 — the session tree

Tab 0 is a Textual app (`--sidebar`), and it is the only tab without a ✕:
closing it would take the sidebar with it.

| Key | Does |
|---|---|
| `Enter` | Open the highlighted conversation as its own tab |
| `s` | Open it **beside** the one you are reading — two conversations, one screen |
| `n` | New Claude session in the highlighted project's directory |
| `t` | Plain terminal tab there — for the git/build/log half of the work |
| `m` | The settings panel (MCP servers) as a tab |
| `x` | Stop a 󰑮 background session (its transcript stays resumable) |
| `r` | Refresh the tree |
| `q` | Close the whole studio |

The tree refreshes itself every 6 s, because it goes stale the moment a
tab is closed or a session starts somewhere else — but only redraws when
something actually changed. It used to rebuild unconditionally, which
collapsed whatever project you had opened and threw the cursor back to
the top, every six seconds. When it does change, expanded projects and
the cursor are put back. Every 30 s it also
re-names open tabs: Claude titles a conversation a little *after* it
starts, so a tab opened from the tree gets its real name on a later pass.

**Opening something already open focuses its tab** rather than starting a
second `claude --resume` against the same conversation.

A running session started *inside* the studio has no Hyprland window of
its own — the parent chain dead-ends at the tmux server — so selecting it
matches its tty against the studio's panes and focuses that tab instead.

## Tabs

One conversation per tab, named from **Claude's own title** for it, not
the directory. Before that, every session in `$HOME` opened a tab called
`santapong` and the close buttons were a guessing game.

- **✕** on a tab closes it. Middle-click anywhere on the tab does the same.
- A tab holding several panes shows `·N` — its name only ever describes
  the first one.
- Closing a tab ends the *terminal*, not the conversation — it is still in
  the tree, still resumable.

tmux has no per-glyph hit testing, so the ✕ carries its own `range=user|`
tag naming the window it belongs to; left-clicking anywhere else on the
tab still just selects it.

## Splits — several panes in one tab

A conversation usually wants something beside it: the test run, the log,
a shell in the same repo.

| | |
|---|---|
| `C-b \|` | split side by side |
| `C-b -` | split stacked |
| `[\|]` `[-]` in the tab bar | the same two, by mouse |
| `C-b ←↑↓→` | move between panes |
| `C-b z` | zoom one pane full-screen, and back |
| mouse | click a pane to focus it, drag a border to resize |

Splits inherit the current pane's directory, so a split next to a session
lands in that session's repo.

**`s` in the tree is the one you want for two conversations at once.** A
bare split gives you a shell; `s` splits the tab you were last reading and
runs `claude --resume` in the new pane, so the two sit side by side. If
that conversation is already open it focuses its tab instead — a second
`claude --resume` on one conversation is the duplicate the tab path
already refuses. With nothing but the tree open there is nothing to sit
beside, so it opens a tab.

**The pane title line only appears once a window actually has two panes.**
A `window-layout-changed` hook turns `pane-border-status` on and off, so a
single-pane tab loses no rows. (`pane-exited` was tried first — it does not
fire for `kill-pane`.)

**The split buttons never cut the sidebar in half.** Clicking `[|]` while
tab 0 is selected opens a terminal *tab* instead: the tree is a Textual app
that owns its whole window. That decision lives in `--split`, which the
tab-bar buttons route through.

## Why MCP settings are not a Studio feature

The studio has no settings framework — it is a tmux session whose tab 0
happens to run a Textual tree. Hypr Settings is the one with the sidebar,
the panes and the `Card` class. Building a second settings surface inside
the studio would mean maintaining two.

But a studio tab is just a tmux window, and Hypr Settings is a terminal
app like the sidebar — so `m` runs `hypr-settings integrations` as a tab.
Same panel, same code, reachable without leaving the studio.

## Theme

`style_tmux()` is idempotent and re-runs on every sidebar start, so a
wallpaper change reaches the tab bar. Colour comes from `hyprdesk.colors()`
— never waybar's raw wallust slots. The ink hierarchy is the fleet's:
`accent2` titles the bar, `accent` marks the selected tab and the active
pane border, `sub` for everything at rest.

Terminal panels get the palette and the ink hierarchy but never the pixel
icons — tmux draws characters, not sprites.

## Files

| | |
|---|---|
| `bin/hypr-claude-studio` | all of it — `launch`, `--sidebar`, `--split` |
| `lib/hyprdesk/claudesessions.py` | where sessions and their titles come from |

Launching when the studio already exists **summons** it to the current
workspace rather than yanking you to wherever it was left.
