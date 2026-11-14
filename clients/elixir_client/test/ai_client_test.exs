defmodule SoliDB.AIClientTest do
  use ExUnit.Case, async: true

  alias SoliDB.AIClient

  # No server: the JSON step is what used to raise UndefinedFunctionError,
  # when mix.exs did not declare Jason.
  test "a request body is encoded as JSON, and none as an empty string" do
    assert AIClient.encode_body(nil) == ""
    assert Jason.decode!(AIClient.encode_body(%{"prompt" => "hi", "n" => 2})) ==
             %{"prompt" => "hi", "n" => 2}
  end

  test "a response body is decoded from a charlist or a binary" do
    assert AIClient.decode_body(~c"{\"id\": \"c1\", \"ok\": true}") == %{"id" => "c1", "ok" => true}
    assert AIClient.decode_body("{\"error\": \"nope\"}") == %{"error" => "nope"}
  end

  test "an empty response body is an empty map" do
    assert AIClient.decode_body(~c"") == %{}
    assert AIClient.decode_body("") == %{}
  end
end
