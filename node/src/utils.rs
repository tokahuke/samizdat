pub struct Chunks<I> {
    it: I,
    size: usize,
    is_error: bool,
    is_done: bool,
}

impl<T, I: Iterator<Item = Result<T, crate::Error>>> Iterator for Chunks<I> {
    type Item = Result<Vec<T>, crate::Error>;
    fn next(&mut self) -> Option<Result<Vec<T>, crate::Error>> {
        if self.is_error || self.is_done {
            return None;
        }

        let mut chunk = Vec::with_capacity(self.size);
        while let Some(item) = self.it.next() {
            match item {
                Ok(item) => {
                    chunk.push(item);
                    if chunk.len() == self.size {
                        return Some(Ok(chunk));
                    }
                }
                Err(error) => {
                    self.is_error = true;
                    return Some(Err(error));
                }
            }
        }

        self.is_done = true;

        Some(Ok(chunk))
    }
}

pub fn chunks<I>(size: usize, it: I) -> Chunks<I>
where
    I: Iterator<Item = Result<u8, crate::Error>>,
{
    Chunks {
        it,
        size,
        is_error: false,
        is_done: false,
    }
}
