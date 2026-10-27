# TODOs

## Open:
Right click also sets anchor. Is this behaviour desired?
Copy log entries as rich text too (severity colors, monospace font), not only as plain text
Scroll bar if list is small: Bigger slider
Scroll bar if list fits into screen: Block slider (make it as big as the space it resides in)
Drag & drop file into Sherlog to open it
Add help text (--help option). Remove the test option "-c" ("CTest").
Minor GUI lag when entire screen is filled with long log lines (has to render too many characters?)
Add context menu
Add "jump to anchor" functionality, either triggered by GUI button or hotkey

Mark malformed entries: broken timestamp, double message in same entry, various parsing issues, etc.
Recognize core dumps in sfile. Warn user about presence of core dumps.
At end: Curoffset problems! Overscroll!
Anchor: Overscroll when anchored to end of small subset
Offsets like `first_offset` and `last_offset` point to non-existing elements if log store is empty. These values aren't options. This is dangerous design and may lead to panics if the log store is empty.
How to render newline chars in log message? Currently they just render as a rectangle.
Search: "Match word" functionality, do not match substring inside word.
Go to date (nearest). Note this is difficult/impossible to implement if the list is not sorted by date, as it becomes ambiguous.
Fold log sources with same name and parent?

Just from looking at a log line, it is hard to tell from which log source it comes. Display log source or colourise it?
Tab support when opening multiple files
Merge multiple sfiles together into the same tree
Save interesting messages into a clue list for quick jumping between them

## Performance, most gain first:
Gains measured or estimated with a 55 MB sfile (1.8 million entries, logs visible after 1.4 s).
The file is parsed while GTK shows the window. Parser gains only shorten the wait for files that take longer to parse than that.

| # | Optimization | Expected gain |
| --- | --- | --- |
| 1 | One parse job per glog file instead of per group. Merge the parts in ring buffer order before the time correction. | ~0.1 s |
| 2 | Schedule parse jobs by uncompressed size times a cost per file type. Glog costs ~5x more per MB than xlog. | part of #1 |
| 3 | Create `LogEntryExt` directly in the parsers, without converting `LogEntry` in `from_source`. | ~0.1 s, less memory |
| 4 | Sort by merging the per-source lists, which are already mostly sorted, or sort indices instead of entries. | ~0.05 s |
| 5 | Glog parser: read in 64 KB chunks instead of one byte per `read()` call. | ~17% of glog parse time |
| 6 | Glog parser: find delimiters with `memchr` and take values as slices instead of copying every byte into `buf`. | up to half of glog parse time |
| 7 | Xlog parser: reuse one line buffer (`read_line`) instead of `lines()`. | small |
| 8 | `draw()`: set the font once per frame and avoid a `String` per date. | smoother scrolling |
| 9 | Anchoring code, `rel_to_abs_offset`, `abs_to_rel_offset` and the render iterator: skip hidden entries with `prev_offset`/`next_offset` in `LogEntryExt`. | faster filtering on big logs |


## Unclear:
Default collapse of TreeView specified by producer (Why by producer? Who is the producer?)
Close file! (File closes just fine, can even be deleted while Sherlog has it open. What was the issue again?)
Copy-Paste logs does not work if we are searching (Seems to work?)

## Done:

Are the timestamps in Xlog always UTC? (yes, this is confirmed)
Copy-paste of log entries: Also paste severity!
Time zone selection via GUI (currently times are always shown in UTC)
File -> Open option with file picker
Figure out exact type of things like SessionId, LogSource, etc. (session ID is a u32 field, log source ID an i32)
Open window with loading screen and only then start parsing sfile, so user gets feedback when he double clicks a large sfile.
