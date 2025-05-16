fn returns(&self, inputs: &[DatumType]) -> Option<DatumType> {
    match inputs {
        [DatumType::Float, DatumType::Float] => Some(DatumType::Float),
        _ => None,
    }
} 