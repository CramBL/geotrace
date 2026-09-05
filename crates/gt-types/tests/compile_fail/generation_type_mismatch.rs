use gt_types::{Versionable, Versioned};

struct Files;

struct Visibility;

impl Versionable for Files {}

impl Versionable for Visibility {}

fn main() {
    let files = Versioned::new(Files);
    let visibility = Versioned::new(Visibility);

    assert_eq!(files.generation(), visibility.generation());
}
