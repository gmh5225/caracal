use caracal::core::core_unit::{CoreOpts, CoreUnit};
use caracal::detectors::{detector::Result, get_detectors};
use std::env;
use std::path::PathBuf;

#[test]
fn test_detectors() {
    insta::glob!("detectors/", "*.cairo", |path| {
        let opts = CoreOpts {
            target: path.to_path_buf(),
            corelib: Some(PathBuf::from(
                env::var("CARGO_MANIFEST_DIR").unwrap() + "/corelib/src",
            )),
        };
        let core = CoreUnit::new(opts).unwrap();
        let mut results = get_detectors()
            .iter()
            .flat_map(|d| d.run(&core))
            .collect::<Vec<Result>>();
        results.sort();
        insta::assert_debug_snapshot!(results);
    });
}
