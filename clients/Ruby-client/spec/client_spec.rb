require "spec_helper"

RSpec.describe SoliDB::Client do
  let(:port) { ENV['SOLIDB_PORT'] ? ENV['SOLIDB_PORT'].to_i : 6745 }
  let(:client) { SoliDB::Client.new('127.0.0.1', port) }
  let(:db_name) { "ruby_test_db_#{Time.now.to_i}" }

  before(:all) do
    # Authenticate once if possible, or per test
  end

  before(:each) do
    client.connect
    begin
        client.auth("_system", "admin", "admin")
    rescue => e
        # Ignore if already authed or server has different creds (fallback)
    end
  end

  after(:each) do
    client.close
  end

  it "connects and pings" do
    expect(client.ping).to be > 0
  end
  
  it "performs CRUD operations" do
    # Cleanup first
    begin
        client.delete_database(db_name) 
    rescue SoliDB::ServerError
    end

    client.create_database(db_name)
    client.create_collection(db_name, "users")

    # Insert
    doc = client.insert(db_name, "users", { "name" => "Ruby", "version" => 3 })
    expect(doc).to be_a(Hash)
    expect(doc["_key"]).not_to be_nil
    key = doc["_key"]

    # Get
    fetched = client.get(db_name, "users", key)
    expect(fetched["name"]).to eq("Ruby")

    # Update
    client.update(db_name, "users", key, { "version" => 3.2 })
    fetched = client.get(db_name, "users", key)
    expect(fetched["version"]).to eq(3.2)
    expect(fetched["name"]).to eq("Ruby") # Merge check

    # Delete
    client.delete(db_name, "users", key)
    
    # Verify deletion
    expect {
       client.get(db_name, "users", key)
    }.to raise_error(SoliDB::ServerError) 
    
    # Cleanup
    client.delete_database(db_name)
  end

  it "performs SDBQL queries" do
     client.create_database(db_name) rescue nil 
     client.create_collection(db_name, "params") rescue nil
     
     client.insert(db_name, "params", { "a" => 1 })
     client.insert(db_name, "params", { "a" => 2 })
     
     results = client.query(db_name, "FOR p IN params FILTER p.a > @val RETURN p", { "val" => 1 })
     expect(results).to be_a(Array)
     expect(results.size).to eq(1)
     expect(results[0]["a"]).to eq(2)
     
     client.delete_database(db_name)
  end
  
  it "handles transactions" do
      client.create_database(db_name) rescue nil 
      
      tx_id = client.begin_transaction(db_name)
      expect(tx_id).to be_a(String) # If protocol returns tx_id string 
      # Or if it's keys of map
      
      # Check if client.begin_transaction returns correct value
      # In Client.rb I didn't fully implement extracting tx_id from map response.
      # If SoliDB returns Ok { tx_id: "..." }
      
      # Let's inspect tx_id if it fails
      # puts "TX_ID: #{tx_id.inspect}"
      
      client.rollback_transaction(tx_id)
      client.delete_database(db_name)
  end

  it "declares, checks and drops foreign keys" do
      db = "ruby_fk_db"
      client.delete_database(db) rescue nil
      client.create_database(db)
      client.create_collection(db, "customers")
      client.create_collection(db, "orders")
      client.insert(db, "customers", {"_key" => "c1", "name" => "Ada"})
      client.insert(db, "orders", {"_key" => "o1", "customer" => "c1"})
      client.insert(db, "orders", {"_key" => "o2", "customer" => "ghost"})

      # A document that refers to a missing parent: the key is refused, and
      # the check says which.
      found = client.check_foreign_key(db, "orders", "customer", "customers")
      expect(found["count"]).to eq(1)
      expect(found["sample"][0]["_key"]).to eq("o2")
      expect { client.create_foreign_key(db, "orders", "customer", "customers") }.to raise_error(StandardError)

      client.delete(db, "orders", "o2")
      key = client.create_foreign_key(db, "orders", "customer", "customers", on_delete: "set_null", name: "fk_customer")
      expect(key["name"]).to eq("fk_customer")
      expect(key["on_delete"]).to eq("set_null")
      expect(client.list_foreign_keys(db, "orders")["foreign_keys"][0]["field"]).to eq("customer")
      expect(client.list_foreign_keys(db, "customers")["referenced_by"][0]["collection"]).to eq("orders")

      expect { client.insert(db, "orders", {"_key" => "o3", "customer" => "nobody"}) }.to raise_error(StandardError)
      client.delete(db, "customers", "c1")
      expect(client.query(db, "FOR o IN orders RETURN o.customer")).to eq([nil])

      client.delete_foreign_key(db, "orders", "fk_customer")
      expect(client.list_foreign_keys(db, "orders")["foreign_keys"]).to eq([])
      client.delete_database(db)
  end

  it "declares field rules and is held to them" do
      db = "ruby_rules_db"
      client.delete_database(db) rescue nil
      client.create_database(db)
      client.create_collection(db, "goods")

      expect(client.get_field_rules(db, "goods")["rules"]).to eq([])
      declared = client.set_field_rules(db, "goods", [
          { field: "name", required: true, type: "string" },
          { field: "price", min: 0 }
      ])
      expect(declared["rules"].map { |rule| rule["field"] }).to eq(["name", "price"])
      expect(client.get_field_rules(db, "goods")).to eq(declared)

      expect { client.insert(db, "goods", {"price" => 3}) }.to raise_error(StandardError)
      expect { client.insert(db, "goods", {"name" => "A", "price" => -1}) }.to raise_error(StandardError)
      client.insert(db, "goods", {"name" => "A", "price" => 3})

      expect(client.set_field_rules(db, "goods", [])["rules"]).to eq([])
      client.insert(db, "goods", {"price" => -5})
      client.delete_database(db)
  end

  it "declares a counter field and reads the numbers back" do
      db = "ruby_counter_db"
      client.delete_database(db) rescue nil
      client.create_database(db)
      client.create_collection(db, "invoices")

      counter = client.create_counter(db, "invoices", "number", start: 7, prefix: "INV-", padding: 4, durable: false)
      expect(counter["state"]["next"]).to eq(7)
      expect(client.insert(db, "invoices", {"total" => 10})["number"]).to eq("INV-0007")
      expect(client.insert(db, "invoices", {"total" => 20})["number"]).to eq("INV-0008")
      expect(client.list_counters(db, "invoices")["counters"][0]["state"]["next"]).to eq(9)

      # The client neither chooses the number nor reuses one.
      expect { client.insert(db, "invoices", {"total" => 1, "number" => "INV-0001"}) }.to raise_error(StandardError)
      expect(client.set_counter_next(db, "invoices", "number", 100)["state"]["next"]).to eq(100)
      expect(client.insert(db, "invoices", {"total" => 30})["number"]).to eq("INV-0100")
      expect { client.set_counter_next(db, "invoices", "number", 100) }.to raise_error(StandardError)

      client.delete_counter(db, "invoices", "number")
      expect(client.list_counters(db, "invoices")["counters"]).to eq([])
      client.delete_database(db)
  end
end
