use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub(crate) type TagIcons = BTreeMap<String, TagIcon>;

// Keep the sidebar artwork, editor symbol and search terms together.
macro_rules! catalog {
    ($($variant:ident: $glyph:literal, $symbol:literal, $label:literal, $($word:literal)|+;)*) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum TagIcon {
            $($variant,)*
            #[serde(other)]
            Hash,
        }

        impl TagIcon {
            pub(crate) const ALL: &[Self] = &[Self::Hash, $(Self::$variant,)*];

            pub(crate) fn glyph(self) -> &'static str {
                match self {
                    Self::Hash => "hash",
                    $(Self::$variant => $glyph,)*
                }
            }

            pub(crate) fn symbol(self) -> &'static str {
                match self {
                    Self::Hash => "number",
                    $(Self::$variant => $symbol,)*
                }
            }

            pub(crate) fn label(self) -> &'static str {
                match self {
                    Self::Hash => "Cancelletto",
                    $(Self::$variant => $label,)*
                }
            }

            pub(crate) fn matches(self, query: &str) -> bool {
                if query.is_empty() {
                    return true;
                }
                match self {
                    Self::Hash => ["cancelletto", "hashtag", "#", "hash"].iter().any(|word| word.contains(query)),
                    $(Self::$variant => [$label, $glyph, $($word),+].iter().any(|word| word.to_lowercase().contains(query)),)*
                }
            }

            pub(crate) fn automatic(tag: &str) -> Self {
                match tag.rsplit('/').next().unwrap_or(tag) {
                    $($($word)|+ => Self::$variant,)*
                    _ => Self::Hash,
                }
            }
        }
    };
}

catalog! {
    Home: "house", "house.fill", "Casa", "casa" | "home";
    Heart: "heart", "heart.fill", "Cuore", "cuore" | "amore" | "love";
    People: "users", "person.2.fill", "Persone", "persone" | "famiglia" | "family";
    Gift: "gift", "gift.fill", "Regalo", "regali" | "gifts";
    Star: "star", "star.fill", "Stella", "preferiti" | "favorites";
    Trophy: "trophy", "trophy.fill", "Trofeo", "premi" | "awards";
    Study: "graduation-cap", "graduationcap.fill", "Studio", "studio" | "scuola" | "school";
    Clock: "clock", "clock.fill", "Orologio", "tempo" | "time";
    Health: "heart-pulse", "heart.text.square.fill", "Salute", "salute" | "health";
    Shopping: "shopping-cart", "cart.fill", "Spesa", "spesa" | "shopping";
    Clothes: "shirt", "tshirt.fill", "Vestiti", "vestiti" | "clothes";
    Run: "footprints", "figure.run", "Corsa", "corsa" | "running";
    Fitness: "dumbbell", "dumbbell.fill", "Allenamento", "allenamento" | "fitness";
    Food: "utensils", "fork.knife", "Cucina", "cucina" | "ricette" | "recipes" | "food";
    Coffee: "coffee", "cup.and.saucer.fill", "Caffè", "caffè" | "coffee";
    Cake: "cake", "birthday.cake.fill", "Torta", "compleanni" | "birthdays";
    Plane: "plane", "airplane", "Aereo", "viaggi" | "viaggio" | "travel";
    Luggage: "luggage", "suitcase.rolling.fill", "Valigia", "bagagli" | "luggage";
    Map: "map", "map.fill", "Mappa", "mappe" | "maps";
    Mountain: "mountain", "mountain.2.fill", "Montagna", "montagna" | "mountains";
    Bike: "bike", "bicycle", "Bicicletta", "bici" | "cycling";
    Car: "car-front", "car.fill", "Auto", "auto" | "car";
    Sun: "sun", "sun.max.fill", "Sole", "sole" | "sun";
    Moon: "moon", "moon.fill", "Luna", "luna" | "moon";
    Leaf: "leaf", "leaf.fill", "Foglia", "natura" | "nature";
    Flower: "flower-2", "fleuron.fill", "Fiore", "fiori" | "flowers";
    Paw: "paw-print", "pawprint.fill", "Zampa", "animali" | "pets";
    Ideas: "lightbulb", "lightbulb.fill", "Lampadina", "idee" | "ideas";
    Book: "book-open", "book.fill", "Libro", "libri" | "books" | "lettura" | "reading";
    Writing: "pen-line", "pencil", "Penna", "scrittura" | "writing";
    Journal: "notebook", "book.closed.fill", "Diario", "diario" | "journal";
    Calendar: "calendar-days", "calendar", "Calendario", "oggi" | "today" | "calendario" | "calendar";
    Todo: "square-check", "checkmark.square.fill", "Da fare", "todo" | "checklist";
    Work: "briefcase", "briefcase.fill", "Valigetta", "lavoro" | "work" | "business";
    Folder: "folder", "folder.fill", "Cartella", "progetti" | "projects";
    Chart: "chart-no-axes-column", "chart.bar.fill", "Grafico", "statistiche" | "analytics";
    Wallet: "wallet", "wallet.bifold.fill", "Portafoglio", "finanze" | "finance";
    Code: "code", "chevron.left.forwardslash.chevron.right", "Codice", "codice" | "programmazione" | "coding";
    Terminal: "terminal", "terminal.fill", "Terminale", "terminale" | "shell";
    Robot: "bot", "cpu.fill", "Robot", "robot" | "ml";
    Music: "music", "music.note", "Musica", "musica" | "music";
    Headphones: "headphones", "headphones", "Cuffie", "podcast" | "audio";
    Camera: "camera", "camera.fill", "Fotocamera", "foto" | "photography";
    Film: "film", "film.fill", "Cinema", "film" | "movies";
    Game: "gamepad-2", "gamecontroller.fill", "Videogiochi", "giochi" | "games";
    Paint: "palette", "paintpalette.fill", "Tavolozza", "arte" | "art" | "design";
    Globe: "globe", "globe", "Globo", "lingue" | "languages";
    Mail: "mail", "envelope.fill", "Posta", "posta" | "mail";
    Flag: "flag", "flag.fill", "Bandiera", "obiettivi" | "goals";
    Bookmark: "bookmark", "bookmark.fill", "Segnalibro", "segnalibri" | "bookmarks";
    Key: "key-round", "key.fill", "Chiave", "chiavi" | "keys";
    Lock: "lock", "lock.fill", "Lucchetto", "privacy" | "segreti";
    Sailboat: "sailboat", "sailboat.fill", "Barca a vela", "mare" | "sailing";
    Bolt: "zap", "bolt.fill", "Fulmine", "energia" | "energy";
    Astronomy: "moon-star", "moon.stars.fill", "Astronomia", "spazio" | "space";
    Sparkles: "sparkles", "sparkles", "Scintille", "ispirazione" | "inspiration";
    Beaker: "flask-conical", "flask.fill", "Laboratorio", "scienza" | "science";
    Compass: "compass", "safari.fill", "Bussola", "esplorazione" | "exploration";
    Umbrella: "umbrella", "umbrella.fill", "Ombrello", "pioggia" | "rain";
}

pub(crate) fn resolve(icons: &TagIcons, tag: &str) -> TagIcon {
    icons
        .get(tag)
        .copied()
        .unwrap_or_else(|| TagIcon::automatic(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_icons_override_automatic_icons_per_tag() {
        let mut icons = TagIcons::new();
        let tag = "progetti/viaggi";
        assert_eq!(resolve(&icons, tag), TagIcon::Plane);
        icons.insert(tag.into(), TagIcon::Hash);
        assert_eq!(resolve(&icons, tag), TagIcon::Hash);
        assert_eq!(resolve(&icons, "viaggi"), TagIcon::Plane);
        icons.remove(tag);
        assert_eq!(resolve(&icons, tag), TagIcon::Plane);
        assert!(TagIcon::Plane.matches("aer"));
    }
}
