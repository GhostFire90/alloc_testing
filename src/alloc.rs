use crate::meta_utils::{ManagedBytes, MetaData};
use core::alloc::{Allocator, GlobalAlloc, Layout};

const PAGE_SIZE: usize = 4096;

fn get_page<'a>() -> ManagedBytes<'a>
{
  unsafe {
    let layout = Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE);
    let ptr = std::alloc::System.alloc(layout);
    ManagedBytes::new(ptr, layout)
  }
}

struct MetaAllocInner {}

#[cfg(test)]
mod meta_tests
{
  const ALLOC_COUNT: usize = 1000;

  use core::alloc::{Allocator, Layout};
  use core::ptr::NonNull;

  use crate::MetaAlloc;

  #[test]
  pub fn fifo_alloc()
  {
    let myalloc: MetaAlloc = MetaAlloc::new();
    let mut stored: Vec<NonNull<[u8]>> = Vec::with_capacity(ALLOC_COUNT);
    for _ in 0..ALLOC_COUNT
    {
      stored.push(
        myalloc
          .allocate(Layout::from_size_align(32, 16).expect("layout failed"))
          .expect("ALLOCATION FAILURE"),
      );
    }
    for _ in 0..ALLOC_COUNT
    {
      unsafe {
        myalloc.deallocate(
          stored
            .pop()
            .map(|x| NonNull::<u8>::new_unchecked(x.as_ptr() as *mut u8))
            .expect("SOMEHOW THE WRONG AMOUNT"),
          Layout::from_size_align(32, 16).expect("layout failed"),
        );
      }
    }
  }

  #[test]
  pub fn lifo_alloc()
  {
    let myalloc: MetaAlloc = MetaAlloc::new();
    let mut stored: Vec<NonNull<[u8]>> = Vec::with_capacity(ALLOC_COUNT);
    for _ in 0..ALLOC_COUNT
    {
      stored.push(
        myalloc
          .allocate(Layout::from_size_align(32, 16).expect("layout failed"))
          .expect("ALLOCATION FAILURE"),
      );
    }
    stored.iter().rev().for_each(|x| {
      let p_actual = unsafe { NonNull::<u8>::new_unchecked(x.as_ptr() as *mut u8) };
      unsafe {
        myalloc.deallocate(
          p_actual,
          Layout::from_size_align(32, 16).expect("layout failed"),
        );
      }
    });
  }
}
