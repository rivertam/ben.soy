//! Drum-cover metadata shared by the `/drums` gallery and the logbook.
//! Keep the list newest first and use YouTube's publication date, not the
//! date the cover was added to the site.

#[derive(Clone, Copy)]
pub struct DrumCover {
    pub youtube_id: &'static str,
    pub watch_url: &'static str,
    pub published: &'static str,
    pub title: &'static str,
    pub artist: &'static str,
    pub card_label: &'static str,
    pub log_link_label: &'static str,
}

pub static DRUM_COVERS: [DrumCover; 4] = [
    DrumCover {
        youtube_id: "MdiiqG8hzOg",
        watch_url: "https://www.youtube.com/watch?v=MdiiqG8hzOg",
        published: "2026-08-25",
        title: "Jolene",
        artist: "Dolly Parton",
        card_label: "Dolly Parton cover →",
        log_link_label: "Dolly Parton ↗",
    },
    DrumCover {
        youtube_id: "HyPCqzi74nE",
        watch_url: "https://www.youtube.com/watch?v=HyPCqzi74nE",
        published: "2026-08-23",
        title: "I Bet You Look Good on the Dancefloor",
        artist: "Arctic Monkeys",
        card_label: "Arctic Monkeys cover →",
        log_link_label: "Arctic Monkeys ↗",
    },
    DrumCover {
        youtube_id: "8lrjsP1KWrY",
        watch_url: "https://www.youtube.com/watch?v=8lrjsP1KWrY",
        published: "2023-11-08",
        title: "The Sunshine",
        artist: "Manchester Orchestra",
        card_label: "Manchester Orchestra cover →",
        log_link_label: "Manchester Orchestra ↗",
    },
    DrumCover {
        youtube_id: "VaKI7J2M2Ms",
        watch_url: "https://www.youtube.com/watch?v=VaKI7J2M2Ms",
        published: "2023-08-17",
        title: "I Knew You Were Trouble (Taylor's Version)",
        artist: "Taylor Swift",
        card_label: "Taylor Swift cover →",
        log_link_label: "Taylor Swift ↗",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn iso_date(date: &str) -> bool {
        let bytes = date.as_bytes();
        date.len() == 10
            && bytes.iter().enumerate().all(|(i, byte)| match i {
                4 | 7 => *byte == b'-',
                _ => byte.is_ascii_digit(),
            })
    }

    #[test]
    fn metadata_is_complete_unique_and_newest_first() {
        for cover in DRUM_COVERS.iter() {
            assert!(iso_date(cover.published), "bad date: {}", cover.published);
            assert!(!cover.youtube_id.is_empty());
            assert!(!cover.title.is_empty());
            assert!(!cover.artist.is_empty());
            assert!(!cover.card_label.is_empty());
            assert!(!cover.log_link_label.is_empty());
            assert_eq!(
                cover.watch_url,
                format!("https://www.youtube.com/watch?v={}", cover.youtube_id)
            );
        }
        for pair in DRUM_COVERS.windows(2) {
            assert!(pair[0].published > pair[1].published);
        }
        for (index, cover) in DRUM_COVERS.iter().enumerate() {
            assert!(
                DRUM_COVERS[index + 1..]
                    .iter()
                    .all(|other| other.youtube_id != cover.youtube_id),
                "duplicate YouTube id {}",
                cover.youtube_id
            );
        }
    }
}
