require "spec_helper"

# Transactions over the driver, against a running server (card #793):
#
#   SOLIDB_PORT=6745 rspec spec/transaction_spec.rb
#
# The client used to send a transaction's writes as plain commands, applied
# at once and kept after a rollback, and its default pool of four
# connections lost the login on three of them.
RSpec.describe "SoliDB::Client transactions" do
  port = ENV["SOLIDB_PORT"] ? ENV["SOLIDB_PORT"].to_i : 6745
  user = ENV.fetch("SOLIDB_USER", "admin")
  password = ENV.fetch("SOLIDB_PASSWORD", "admin")
  db = "ruby_tx_test_db"

  def self.connected(port, user, password)
    c = SoliDB::Client.new("127.0.0.1", port) # the default pool of four
    c.connect
    c.auth("_system", user, password)
    c
  end

  let(:client) { self.class.connected(port, user, password) }
  let(:observer) { self.class.connected(port, user, password) }

  before do
    begin
      client.delete_database(db)
    rescue SoliDB::ServerError
      nil
    end
    client.create_database(db)
    client.create_collection(db, "docs")
    client.insert(db, "docs", { "n" => 1 }, "base")
    client.insert(db, "docs", { "n" => 1 }, "gone")
  end

  after do
    client.delete_database(db)
    client.close
    observer.close
  end

  def n(c, db, key)
    c.get(db, "docs", key)["n"]
  rescue SoliDB::ServerError
    nil
  end

  it "leaves nothing after a rollback, and shows nothing before it" do
    tx = client.begin_transaction(db)
    expect(tx).to be_a(String)
    expect(client.in_transaction?).to be(true)
    client.insert(db, "docs", { "n" => 7 }, "new")
    client.update(db, "docs", "base", { "n" => 2 })
    client.delete(db, "docs", "gone")
    expect(n(observer, db, "new")).to be_nil
    expect(n(observer, db, "base")).to eq(1)
    client.rollback_transaction(tx)
    expect(client.in_transaction?).to be(false)
    expect(n(observer, db, "new")).to be_nil
    expect(n(observer, db, "base")).to eq(1)
    expect(n(observer, db, "gone")).to eq(1)
  end

  it "applies every write at once at the commit" do
    tx = client.begin_transaction(db)
    client.insert(db, "docs", { "n" => 3 }, "a")
    # Reads outside the transaction go out on the other pooled connections.
    expect(client.get(db, "docs", "base")["n"]).to eq(1)
    client.update(db, "docs", "base", { "n" => 2 })
    client.delete(db, "docs", "gone")
    expect(client.query(db, "FOR d IN docs FILTER d._key == 'base' RETURN d.n")).to eq([1])
    expect(n(observer, db, "a")).to be_nil
    client.commit_transaction(tx)
    expect(n(observer, db, "a")).to eq(3)
    expect(n(observer, db, "base")).to eq(2)
    expect(n(observer, db, "gone")).to be_nil
  end

  it "stages a query that writes and refuses a second begin" do
    tx = client.begin_transaction(db)
    client.query(db, "INSERT {_key: 'q', n: 1} INTO docs")
    expect(n(observer, db, "q")).to be_nil
    expect { client.begin_transaction(db) }.to raise_error(SoliDB::TransactionError)
    client.rollback_transaction(tx)
    expect(n(observer, db, "q")).to be_nil
    expect { client.commit_transaction(tx) }.to raise_error(SoliDB::TransactionError)
  end

  it "logs every pooled connection in" do
    8.times { expect(client.list_databases).to include(db) }
  end
end
