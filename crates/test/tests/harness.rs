use goethite_test::{TestFailure, TestSuite, TestSummary, current_test, run_suites_with_filter};

mod sample_suite {
    use goethite_test::{assert, assert_eq, assert_matches, assert_ne, test_suite};

    test_suite! {
        fn passing_assertions() {
            assert!(true);
            assert_eq!(2 + 2, 4);
            assert_ne!(1, 2);
            assert_matches!(Some(7), Some(n) if n > 5);
        }

        fn failing_assert_eq() {
            assert_eq!(10, 20, "math broke");
        }

        fn failing_assert_matches() {
            assert_matches!(Option::<u32>::None, Some(_));
        }

        #[ignore]
        fn skipped_case() {
            assert!(false, "ignored tests must not run");
        }
    }
}

#[test]
fn failure_truncates_on_utf8_boundary_and_formats_location() {
    let long = "汉".repeat(300); // 900 bytes > MAX_MESSAGE (512)
    let failure = TestFailure::at("demo.rs", 42, format_args!("{long}"));
    assert!(failure.as_str().len() <= 512);
    assert!(failure.as_str().chars().all(|c| c == '汉'));
    assert_eq!(failure.file(), "demo.rs");
    assert_eq!(failure.line(), 42);
    assert!(format!("{failure}").starts_with("demo.rs:42: 汉"));
}

#[test]
fn run_suites_reports_passed_failed_and_ignored() {
    let mut output = String::new();
    let summary = run_suites_with_filter(&[sample_suite::suite()], None, &mut output);

    assert_eq!(
        summary,
        TestSummary {
            passed: 1,
            failed: 2,
            ignored: 1,
        }
    );
    assert!(!summary.is_success());
    assert_eq!(current_test(), "");
    assert!(output.contains("[       OK ]"));
    assert!(output.contains("[  FAILED  ]"));
    assert!(output.contains("[ SKIPPED  ]"));
    assert!(output.contains("math broke"));
    assert!(output.contains("harness.rs:"));
    assert!(output.contains("GOETHITE_TEST_SUMMARY passed=1 failed=2 ignored=1"));
}

#[test]
fn filter_selects_matching_tests_by_name_or_suite() {
    let mut output = String::new();
    let summary = run_suites_with_filter(
        &[sample_suite::suite()],
        Some("passing_assertions"),
        &mut output,
    );

    assert_eq!(
        summary,
        TestSummary {
            passed: 1,
            failed: 0,
            ignored: 0,
        }
    );
    assert!(summary.is_success());
    assert!(output.contains("filter: \"passing_assertions\""));
    assert!(output.contains("GOETHITE_TEST_SUMMARY passed=1 failed=0 ignored=0"));

    let empty = TestSuite::new("other_suite", &[]);
    let mut output = String::new();
    let summary = run_suites_with_filter(&[empty, sample_suite::suite()], Some("skipped"), &mut output);
    assert_eq!(
        summary,
        TestSummary {
            passed: 0,
            failed: 0,
            ignored: 1,
        }
    );
}
