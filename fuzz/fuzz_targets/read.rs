//! Runs every reader of stated values on every instance of arbitrary bytes.
//!
//! Invariants: no reader panics, whatever the instance; what a reader
//! returns is finite (every unit factor and uncertainty is positive and
//! finite), and every error formats. Each later reader is added here.

#![no_main]

use libfuzzer_sys::fuzz_target;
use stepq::model::Graph;
use stepq::p21::parse;
use stepq::units::{ContextUnits, Unit, UnitError, representation_context, unit};

fuzz_target!(|data: &[u8]| {
    let Ok(exchange) = parse(data) else {
        return;
    };
    let graph = Graph::build(exchange);
    let ids: Vec<u64> = graph.exchange().instances().iter().map(|i| i.id).collect();
    for &id in &ids {
        check_unit(unit(&graph, id));
        if let Err(error) = representation_context(&graph, id) {
            let _ = error.to_string();
        }
        for units in [
            ContextUnits::of_context(&graph, id),
            ContextUnits::of_representation(&graph, id),
            ContextUnits::of_item(&graph, id),
        ] {
            match units {
                Ok(units) => check_context(&units),
                Err(error) => {
                    let _ = error.to_string();
                }
            }
        }
    }
});

fn check_unit(result: Result<Unit, UnitError>) {
    match result {
        Ok(unit) => assert!(
            unit.factor.is_finite() && unit.factor > 0.0,
            "#{}: factor {}",
            unit.instance,
            unit.factor
        ),
        Err(error) => {
            let _ = error.to_string();
        }
    }
}

fn check_context(units: &ContextUnits) {
    for unit in [units.length(), units.plane_angle(), units.solid_angle()] {
        check_unit(unit.cloned());
    }
    match units.uncertainty() {
        Ok(uncertainties) => {
            for uncertainty in uncertainties {
                assert!(
                    uncertainty.value.is_finite() && uncertainty.value > 0.0,
                    "#{}: uncertainty {}",
                    uncertainty.instance,
                    uncertainty.value
                );
                check_unit(Ok(uncertainty.unit.clone()));
            }
        }
        Err(error) => {
            let _ = error.to_string();
        }
    }
}
