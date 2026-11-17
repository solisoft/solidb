# SoliDB driver for iyi

The driver lives in its own repository,
[github.com/solisoft/solidb_iyi](https://github.com/solisoft/solidb_iyi):
`iyi get` fetches a package from a repository whose root holds its
`iyi.mod`, and versions are that repository's tags, so it cannot live in a
subdirectory of this one.

```sh
iyi get github.com/solisoft/solidb_iyi --as solidb
```

```crystal
import solidb::{Client}

db = Client.new("127.0.0.1", 6745)
db.auth("_system", "admin", "secret").or_panic
```

Its README has the calls, and the documentation site a page:
`doc/app/views/docs/clients-iyi.html.slv`.
