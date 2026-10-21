use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

impl<T> Page<T> {
    #[must_use]
    pub fn new(items: Vec<T>, total: u64, page: u32, page_size: u32) -> Self {
        Self {
            items,
            total,
            page,
            page_size,
        }
    }

    #[must_use]
    pub fn total_pages(&self) -> u32 {
        if self.page_size == 0 {
            0
        } else {
            let total = self.total;
            let size = u64::from(self.page_size);
            let pages = total.div_ceil(size);
            u32::try_from(pages).unwrap_or(u32::MAX)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub page: u32,
    pub page_size: u32,
}

impl PageRequest {
    #[must_use]
    pub const fn new(page: u32, page_size: u32) -> Self {
        Self { page, page_size }
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        if self.page == 0 {
            0
        } else {
            ((self.page - 1) as u64) * (self.page_size as u64)
        }
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.page_size as u64
    }
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_computes_pagination() {
        let p = Page::new(vec![1, 2, 3], 10, 1, 3);
        assert_eq!(p.total_pages(), 4);
        assert_eq!(p.items.len(), 3);

        let req = PageRequest::new(2, 10);
        assert_eq!(req.offset(), 10);
        assert_eq!(req.limit(), 10);
    }
}
