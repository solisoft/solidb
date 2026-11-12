# 2026-11-12: the whole comparison on the published 2.9.5 image

`run.sh all` then `bank.sh postgres solidb-main` on rocky-build2, SoliDB from
`ghcr.io/solisoft/solidb:v2.9.5` (`image.digest`). Another session compiled on
the machine during part of it, so read the directories as follows.

- **This directory**: the first pass. `noise.log` has the load and the number
  of `rustc` processes every minute. Compilers ran from 07:35 to 08:32 UTC
  (`solidb-solistore`, `postgres`, `postgres-typed`, `mongo`) and from 09:35
  to 09:48 (the bank, in `bank/`): those are disturbed. `arango`, `surreal`
  and the five `-durable` configurations ran with no compiler.
- **`quiet/`**: the disturbed configurations again, each started after a
  minute with no compiler and repeated when one ran during it (`noise.log`
  in each attempt lists the 10 s samples with a compiler running).
  - `postgres-2`, `postgres-typed-1`, `mongo-1`: no compiler.
  - `postgres-1` (bank files) and `solidb-main-1` (the bank on 2.9.5): no
    compiler. `postgres-1` also holds the disturbed first attempt of the
    `postgres` comparison.
  - `solidb-solistore-1` to `-4`: all four disturbed.
- **`quiet2/`**: `solidb-solistore` again; disturbed as well.

No pass of `solidb-solistore` is quiet from end to end. The figures quoted
for it take, per series, the attempts whose three runs agree within 10%
(seven attempts counting the first pass) and the median of those. `insert`
at 32 clients has no such attempt (least spread: 16%).
