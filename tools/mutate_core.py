import subprocess, sys, pathlib

root = pathlib.Path(sys.argv[1])
f = root / "src/player/core.rs"
orig = f.read_text()
mutations = [
    ("stale end-file", "        if live != load {\n            return;\n        }", ""),
    ("retry once", "Attempt::First => Some(Attempt::Retry),", "Attempt::First => None,"),
    ("warm cap", "if failed || self.in_flight.len() >= RESOLVE_CAP {", "if failed {"),
    ("negative cache", "let failed = matches!(self.entries.get(id), Some(Cached::Failed { until }) if *until > now);", "let failed = false;"),
    ("stale resolve", "        if track.id != id {\n            return;\n        }", ""),
    ("prefetch after audio", "        if self.audible()\n            && let Some", "        if true\n            && let Some"),
    ("b restarts", "&& clock.at(now) > RESTART_AFTER", "&& clock.at(now) > Duration::MAX"),
    ("station stops", "                        fx.push(notice(Level::Error, format!(\"{} is off the air: {reason}\", station.name)));\n                        self.stop(fx);", "                        self.skip(true, now, fx);"),
    ("remove current", "if self.queue.remove(id) == Removed::Current && was_active {", "if self.queue.remove(id) == Removed::Current && was_active && false {"),
    ("refill on play", "        if !matches!(self.playback, Playback::Idle)\n            && let Some(seed)", "        if self.audible()\n            && let Some(seed)"),
]
q = root / "src/player/queue.rs"
qorig = q.read_text()
qmut = [("dry refill", "*fetch = if added == 0 { Fetch::Dry { at: seed } } else { Fetch::Idle };", "let _ = seed; *fetch = Fetch::Idle;")]
try:
    for name, a, b in mutations + qmut:
        target, text = (q, qorig) if (name, a, b) in qmut else (f, orig)
        assert a in text, name
        target.write_text(text.replace(a, b, 1))
        r = subprocess.run(["cargo", "test", "--lib", "-q"], cwd=root, capture_output=True, text=True)
        failed = sorted({l.strip() for l in r.stdout.splitlines() if l.startswith("    player::") or l.startswith("    domain")}); failed = failed or [r.stderr[-300:]]
        status = "caught" if r.returncode != 0 else "MISSED"
        print(f"{status:7} {name}: {len(failed)} failing {[l.split("::")[-1] for l in failed][:3]}")
        target.write_text(text)
finally:
    f.write_text(orig)
    q.write_text(qorig)
