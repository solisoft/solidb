defmodule SoliDB.MixProject do
  use Mix.Project

  def project do
    [
      app: :solidb,
      version: "0.1.0",
      elixir: "~> 1.12",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  defp deps do
    [
      {:msgpax, "~> 2.3"}
    ]
  end
end
