# Defect records

One file per defect found in use, each stating what was measured, what caused
it and the change that closed it. They are records of a failure and its fix,
not design essays (`M-NO-META-DESIGN-DOCUMENTATION`): each one is written once,
when the fix lands, and is not revised afterwards.

Nothing runs them. They exist because a forensic tool's own failures are worth
the same treatment as the evidence it handles — measured, attributed and kept —
and because the numbers in them are the calibration behind constants in the
code, which cite these files by name (`A-SUPPORT-DECLARED`).

The measurements come from a scan of a 1 TB mechanical disk of ten years' use,
whose manifest held 345,862 records.
