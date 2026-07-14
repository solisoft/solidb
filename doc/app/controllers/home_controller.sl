# Home controller - handles the root routes

class HomeController < Controller
    # GET /
    def index
        render("home/index", {
            "title": "SoliDB — the multi-model database written in Rust"
        })
    end

    # GET /health
    def health
        {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": "{\"status\":\"ok\"}"
        }
    end
end
