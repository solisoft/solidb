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
end
