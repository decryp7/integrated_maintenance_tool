#!/usr/bin/env python3
"""Checks the plan gate of the Gitea workflows: every job except `plan` (and the *-mark jobs, which gate on the
schedule) needs `plan` and tests its `run` output; every `needs` names an existing job. Needs PyYAML (pip install pyyaml); a local check, not wired into CI.
Usage: check-workflows.py FILE..."""
import sys
import yaml

GATE = "needs.plan.outputs.run == 'true'"
errors = []
for path in sys.argv[1:]:
    doc = yaml.safe_load(open(path))
    jobs = doc["jobs"]
    if "plan" not in jobs:
        errors.append(f"{path}: no plan job")
        continue
    if "run" not in jobs["plan"].get("outputs", {}):
        errors.append(f"{path}: plan has no `run` output")
    for name, job in jobs.items():
        needs = job.get("needs", [])
        needs = [needs] if isinstance(needs, str) else needs
        for n in needs:
            if n not in jobs:
                errors.append(f"{path}: {name} needs unknown job {n}")
        if name == "plan":
            continue
        if name.endswith("-mark"):
            if "success()" not in str(job.get("if", "")):
                errors.append(f"{path}: {name} must gate on success()")
            continue
        if "plan" not in needs:
            errors.append(f"{path}: {name} does not need plan")
        if GATE not in str(job.get("if", "")):
            errors.append(f"{path}: {name} does not test the plan output")
print("\n".join(errors) or "ok: " + ", ".join(sys.argv[1:]))
sys.exit(1 if errors else 0)
