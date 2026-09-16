# zbxpatrol Interactive Wizard Guide (English)

> The zero-learning-curve entry for everyone: number/letter selection, `?` help, Tab completion, level jumps.
> 中文版: [INTERACTIVE.zh.md](INTERACTIVE.zh.md)

<a id="toc"></a>
## Contents

1. [Entry & first-run setup](#start)
2. [Universal inputs](#input) (? / q / exit / m·g·h / empty Enter / Tab)
3. [Level structure & path prefix](#levels)
4. [Browse: groups → hosts → metrics](#browse) — [Host context](#hostmenu) · [Chart continuous mode](#chart) · [Query keys](#hostquery)
5. [Generate report](#report)
6. [Query metrics](#query)
7. [List operations: filter/page/multi-select/name](#lists)

---

<a id="start"></a>
## 1. Entry & first-run setup

Run in a terminal without a subcommand:

```bash
zbxpatrol        # or ./zbxpatrol
```

- Works in a real TTY only; auto-disabled (with a hint) when piped or invoked by programs — see the CLI doc.
- **First run** with no config enters setup: prompts for Zabbix URL, username, password (hidden), skip-TLS choice, timezone; after **connectivity is verified** the config is saved to `~/.zbxpatrol/config.env` (mode 600). All later commands need no configuration.

[↑ TOC](#toc)

<a id="input"></a>
## 2. Universal inputs (any level)

| Input | Action |
|---|---|
| `1`, `2`… or `a`, `b`… (a=1) | Pick the numbered menu item |
| `?` | Show help for this level (valid inputs + commands), then re-print the gray "input:" hint line |
| `q` | Go back one level |
| `exit` / `quit` | **Quit the whole wizard** from any level |
| `m` | Jump to main menu (in browse mode) |
| `g` | Jump to group level (browse mode) |
| `h` | Jump to host-list level (browse mode) |
| Empty Enter | Silent re-prompt (no error) |
| `Ctrl+C` | Force quit |

**Tab completion**: press Tab to list candidates (**max 12 shown**, keeps the screen clean); a unique candidate completes automatically. Candidates are **data items only** (group / host / item-key names) sourced from Zabbix and cached for the session. A gray "可输入/Input:" hint line above each prompt explains what can be typed.

[↑ TOC](#toc)

<a id="levels"></a>
## 3. Level structure & path prefix

The prompt always carries the level path, e.g.:

```
[主菜单/Main] >
[主菜单>Browse>All hosts] >
[主菜单>…><host>] newkey >
```

Main menu:

```
 1) Browse data (groups → hosts → metrics)
 2) Generate inspection report
 3) Query metrics
 4) Connectivity self-check
```

[↑ TOC](#toc)

<a id="browse"></a>
## 4. Browse data (drill-down)

### 4.1 Groups → hosts

- **Group list**: "All hosts" + every group, paged (50/page);
- Picking a group opens the **host list** (with IP and OS badge `[Linux]/[Windows]`), paged/filterable;
- Lists are **cached for the session**: loaded once, instant on later navigation.

### <a id="hostmenu"></a>4.2 Host context (stay here after picking a host)

```
Host <host>
 1) View items (filter/page)
 2) Trend chart (CPU/memory/disk/custom item)
 3) Quick inspection (single-host daily, console)
 4) Query custom keys
```

- Every action returns to this menu — switch tasks without re-picking group/host;
- `q` back to host list, `g` back to groups, `m`/`exit` leave.

### <a id="chart"></a>4.3 Trend chart (continuous mode)

1. Pick metric: `1 CPU / 2 memory / 3 disk / 4 custom item` (custom: pick from list or type);
2. Pick time: daily/weekly/monthly/yearly/**custom range**;
3. After rendering, the prompt stays for **typing the next key directly**:
   - fuzzy match: **unique hit renders immediately; multiple hits open a picker** (e.g. `load` lists 3 load items);
   - `1/2/3` quick-switch CPU/memory/disk; `t` change time; `q` back.

### <a id="hostquery"></a>4.4 Query custom keys (continuous mode)

- Type a key (single → smart resolve: unique → query now, multi → picker; comma-separated → literal list);
- `t` changes time; **time is remembered across consecutive queries**; `q` back to host menu.

[↑ TOC](#toc)

<a id="report"></a>
## 5. Generate report

Flow: scope (all/groups/hosts, pre-filterable) → time → strictness → **extra custom items** (skip/pick/typed wildcards) → output mode:

| Option | Result |
|---|---|
| 1 | Console color table + Excel file |
| 2 | Console only (no file) |
| 3 | Excel only |
| 4 | Console + CSV file |
| 5 | Console + JSON file |

Strictness: **loose (baseline 90%) / standard (80%) / strict (70%)** — the baseline is the high-risk line; all percentage thresholds follow (see README scoring table).

[↑ TOC](#toc)

<a id="query"></a>
## 6. Query metrics

1. Key source: **pick from item list** (filter first, then multi-select) or **type** (wildcards `net.if*`);
2. Scope: all/groups/hosts;
3. Time: five presets + custom range;
4. Output: console table (with trend sparklines) / optional CSV save;
5. Afterwards: **continue (same scope)** / change scope / back to main.

[↑ TOC](#toc)

<a id="lists"></a>
## 7. List operations (shared by all paged pickers)

Hint line example: `可输入/Input: number or name (unique→select; otherwise instant filter) | n/p pages | f filter | ? help | exit quit`

| Input | Action |
|---|---|
| Number | Pick the N-th row of the current filtered view |
| Item text (Tab-completable) | **Type to match instantly**: exact/unique prefix/unique contains → selects immediately; **multiple hits → the list is re-filtered on the spot with a hint ("filtered by …, N matches")**; no hit → keeps current filter with a hint |
| `n` / `p` | Next / previous page (50 per page) |
| `f` | Explicitly modify/clear the filter (equivalent to typing directly) |
| `all` (multi-select) | Select all |

**Multi-select** format: `1,3,5` or `all`.

[↑ TOC](#toc) · [Doc index](README.md)
