-- French translations
-- config/locales/fr.lua

return {
  -- Common
  hello = "Bonjour",
  welcome = "Bienvenue, %s !",
  goodbye = "Au revoir",
  
  -- Navigation
  nav = {
    home = "Accueil",
    about = "À propos",
    docs = "Documentation",
    login = "Connexion",
    logout = "Déconnexion",
    signup = "Inscription"
  },
  
  -- Actions
  actions = {
    save = "Enregistrer",
    cancel = "Annuler",
    delete = "Supprimer",
    edit = "Modifier",
    create = "Créer",
    update = "Mettre à jour",
    search = "Rechercher",
    back = "Retour",
    next = "Suivant",
    previous = "Précédent"
  },
  
  -- Messages
  messages = {
    success = "Opération réussie",
    error = "Une erreur s'est produite",
    not_found = "Non trouvé",
    unauthorized = "Vous n'êtes pas autorisé à effectuer cette action",
    confirm_delete = "Êtes-vous sûr de vouloir supprimer ceci ?"
  },
  
  -- Model validation errors
  models = {
    errors = {
      presence = "ne peut pas être vide",
      format = "est invalide",
      acceptance = "doit être accepté",
      inclusion = "n'est pas inclus dans la liste",
      exclusion = "est réservé",
      comparaison = "ne correspond pas",
      numericality = {
        valid_number = "n'est pas un nombre valide",
        valid_integer = "doit être un entier"
      },
      length = {
        eq = "doit faire exactement %d caractères",
        between = "doit faire entre %d et %d caractères",
        minimum = "doit faire au moins %d caractères",
        maximum = "doit faire au plus %d caractères"
      }
    }
  },
  
  -- Date and time formats
  date = {
    formats = {
      default = "%d/%m/%Y",
      short = "%d %b",
      long = "%d %B %Y"
    },
    day_names = {"Dimanche", "Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi"},
    month_names = {"Janvier", "Février", "Mars", "Avril", "Mai", "Juin", "Juillet", "Août", "Septembre", "Octobre", "Novembre", "Décembre"}
  },
  
  time = {
    formats = {
      default = "%H:%M",
      short = "%H:%M",
      long = "%H:%M:%S"
    }
  },
  
  datetime = {
    formats = {
      default = "%d/%m/%Y %H:%M",
      short = "%d %b, %H:%M",
      long = "%d %B %Y à %H:%M"
    }
  },
  
  -- Relative time
  relative_time = {
    now = "à l'instant",
    seconds = "il y a %d secondes",
    minute = "il y a 1 minute",
    minutes = "il y a %d minutes",
    hour = "il y a 1 heure",
    hours = "il y a %d heures",
    day = "hier",
    days = "il y a %d jours",
    week = "il y a 1 semaine",
    weeks = "il y a %d semaines",
    month = "il y a 1 mois",
    months = "il y a %d mois",
    year = "il y a 1 an",
    years = "il y a %d ans"
  },
  
  -- Pagination
  pagination = {
    previous = "Précédent",
    next = "Suivant",
    page = "Page %d sur %d"
  }
}
