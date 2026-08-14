//! Этап 3: Marker Collector.
//!
//! Группирует соседние одинаковые байты первичного потока в маркеры-последовательности:
//! `*` + `*` → `**`, `)` + `)` → `))`, `\n` → `\n`.
//!
//! На этом этапе не принимается решение о семантике: `*` не считается маркером,
//! `}` не считается закрытием. Это только найденные байты и их положение.

use crate::engine::simd::Event;

/// Последовательность одинаковых байтов: полуинтервал `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub start: usize,
    pub end: usize,
    pub byte: u8,
}

impl Marker {
    /// Длина последовательности.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Группирует события в маркеры-последовательности.
pub fn collect(events: &[Event]) -> Vec<Marker> {
    let mut markers = Vec::new();
    let mut index = 0;
    while index < events.len() {
        let event = events[index];
        let mut next = index + 1;
        while next < events.len()
            && events[next].byte == event.byte
            && events[next].position == events[next - 1].position + 1
        {
            next += 1;
        }
        markers.push(Marker {
            start: event.position,
            end: events[next - 1].position + 1,
            byte: event.byte,
        });
        index = next;
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pos: usize, byte: u8) -> Event {
        Event {
            position: pos,
            byte,
        }
    }

    #[test]
    fn groups_adjacent_same_bytes() {
        let events = vec![
            ev(3, b'*'),
            ev(4, b'*'),
            ev(13, b')'),
            ev(14, b')'),
            ev(24, b'\n'),
        ];
        let markers = collect(&events);
        assert_eq!(
            markers,
            vec![
                Marker {
                    start: 3,
                    end: 5,
                    byte: b'*'
                },
                Marker {
                    start: 13,
                    end: 15,
                    byte: b')'
                },
                Marker {
                    start: 24,
                    end: 25,
                    byte: b'\n'
                },
            ]
        );
    }

    #[test]
    fn separates_non_adjacent() {
        let events = vec![ev(1, b'*'), ev(3, b'*')];
        let markers = collect(&events);
        assert_eq!(markers.len(), 2);
        assert_eq!(
            markers[0],
            Marker {
                start: 1,
                end: 2,
                byte: b'*'
            }
        );
        assert_eq!(
            markers[1],
            Marker {
                start: 3,
                end: 4,
                byte: b'*'
            }
        );
    }

    #[test]
    fn empty() {
        assert!(collect(&[]).is_empty());
    }

    #[test]
    fn single_byte_run() {
        let events = vec![ev(0, b'#')];
        let markers = collect(&events);
        assert_eq!(markers[0].len(), 1);
    }
}
