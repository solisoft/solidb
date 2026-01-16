# Official SoliDB Clients

SoliDB provides official client libraries for several popular programming languages. These clients communicate with the database using the efficient MessagePack protocol.

## 📦 Available Clients

| Language | Package Name | Version | Source |
|----------|--------------|---------|--------|
| **Node.js** | `solidb-client` | 0.1.0 | `clients/js-client` |
| **Python** | `solidb` | 0.1.0 | `clients/PYTHON-client` |
| **Go** | `github.com/solisoft/solidb-go-client` | Latest | `clients/go-client` |
| **PHP** | `solidb/php-client` | Latest | `clients/PHP-client` |
| **Ruby** | `solidb` | 0.1.0 | `clients/Ruby-client` |
| **Elixir** | `solidb` | 0.1.0 | `clients/elixir_client` |

## 🚀 Installation & Usage

### Node.js (TypeScript/JavaScript)

```bash
npm install solidb-client
```

```typescript
import { SoliDB } from 'solidb-client';

const db = new SoliDB('http://localhost:6745');
// ...
```

### Python

```bash
pip install solidb
```

```python
from solidb import SoliDB

db = SoliDB("http://localhost:6745")
# ...
```

### Go

```bash
go get github.com/solisoft/solidb-go-client
```

```go
import "github.com/solisoft/solidb-go-client"

client := solidb.NewClient("http://localhost:6745")
// ...
```

### PHP

```bash
composer require solidb/php-client
```

```php
use SoliDB\Client;

$db = new Client('http://localhost:6745');
// ...
```

### Ruby

```bash
gem install solidb
```

```ruby
require 'solidb'

db = SoliDB::Client.new('http://localhost:6745')
# ...
```

### Elixir

Add `solidb` to your `mix.exs` dependencies:

```elixir
def deps do
  [
    {:solidb, "~> 0.1.0"}
  ]
end
```

## 🛠️ Developing Clients

All clients are located in the `clients/` directory of the monorepo. Each client includes its own test suite and benchmarks.

To run benchmarks for all clients:

```bash
cd clients
./run_benchmarks.sh
```
