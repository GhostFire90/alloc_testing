use core::alloc::{AllocError, Allocator, GlobalAlloc, Layout};
use core::ptr::NonNull;
use core::ptr::null_mut;
use core::slice;
use std::sync::Mutex;

const PAGE_SIZE: usize = 4096;

fn get_page() -> *mut u8
{
  unsafe { std::alloc::System.alloc(Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap()) }
}

#[repr(packed)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metadata
{
  pub pad_bytes: usize,
  pub size: usize,
  pub next: *mut Metadata,
}

struct MetaAllocInner
{
  base: *mut Metadata,
  total_size: usize,
}
#[derive(Default)]
pub struct MetaAlloc
{
  inner: Mutex<MetaAllocInner>,
}

unsafe impl Send for MetaAlloc {}
unsafe impl Sync for MetaAlloc {}

impl MetaAlloc
{
  pub const fn new() -> Self
  {
    Self {
      inner: Mutex::new(MetaAllocInner {
        base: null_mut(),
        total_size: 0,
      }),
    }
  }
}

impl Default for MetaAllocInner
{
  fn default() -> Self
  {
    Self {
      base: null_mut(),
      total_size: 0,
    }
  }
}

impl core::fmt::Display for Metadata
{
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
  {
    let pb = self.pad_bytes;
    let sz = self.size;
    let nx = self.next;
    write!(f, "|PB: {}, SZ: {}, NX: {:?}|-> ", pb, sz, nx)?;
    if !nx.is_null()
    {
      write!(f, "{}", unsafe { nx.read() })?;
    }
    Ok(())
  }
}

impl Metadata
{
  pub fn new(size: usize) -> Self
  {
    Self {
      pad_bytes: 0,
      size,
      next: null_mut(),
    }
  }
  pub fn with_padding(mut self, padding: usize) -> Self
  {
    self.pad_bytes = padding;
    self
  }
  pub fn with_next(mut self, ptr: *mut Metadata) -> Self
  {
    self.next = ptr;
    self
  }

  pub fn get_total_size(&self) -> usize
  {
    self.pad_bytes + self.size + size_of::<Self>()
  }

  pub fn split(p_self: *mut Self, needed_size: usize, pad_bytes: usize) -> *mut Self
  {
    unsafe {
      let mut data = *p_self;
      let my_size = data.get_total_size();
      let mut new_node = Metadata::new(needed_size).with_padding(pad_bytes);
      let new_size = my_size - new_node.get_total_size();
      let base = data.get_base(p_self) as *mut Self;
      let next = {
        if new_size > size_of::<Self>()
        {
          base.byte_add(new_node.get_total_size())
        }
        else
        {
          new_node.size = data.size;
          data.next
        }
      };

      if !next.is_null()
      {
        data.size = new_size;
        data.write_to_ptr(next);
      }

      new_node.next = next;
      new_node.write_to_ptr(base)
    }
  }
  pub fn write_to_ptr(self, ptr: *mut Metadata) -> *mut Metadata
  {
    unsafe {
      let addr = ptr.byte_add(self.pad_bytes);
      addr.write(self);
      addr
    }
  }
  pub fn get_base(&self, ptr: *mut Metadata) -> *mut ()
  {
    unsafe { ptr.byte_sub(self.pad_bytes) as *mut () }
  }
}

unsafe impl Allocator for MetaAlloc
{
  fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>
  {
    if let Ok(mut inner) = self.inner.lock()
    {
      let total_size = size_of::<Metadata>() + layout.size();
      let actual_layout = unsafe { Layout::from_size_align_unchecked(total_size, layout.align()) };
      let padded_size = actual_layout.pad_to_align().size();

      if inner.base.is_null() || padded_size > inner.total_size
      {
        let last = inner.base;
        inner.base = get_page() as *mut Metadata;
        if inner.base.is_null()
        {
          return Err(AllocError);
        }
        unsafe {
          inner.base.write(Metadata::new(PAGE_SIZE).with_next(last));
        }
        inner.total_size += PAGE_SIZE;
      }

      let pad_bytes = padded_size - total_size;

      let header = Metadata::new(layout.size()).with_padding(pad_bytes);
      let mut current = inner.base;
      let mut last: *mut Metadata = null_mut();
      while !current.is_null()
      {
        let data = unsafe { *current };
        if data.get_total_size() > header.get_total_size()
        {
          let res = Metadata::split(current, header.size, header.pad_bytes);
          unsafe {
            if last.is_null()
            {
              inner.base = (*res).next;
            }
            else
            {
              (*last).next = (*res).next;
            }
            let data_ptr = res.add(1) as *mut u8;
            inner.total_size -= (*res).size;
            return NonNull::new(slice::from_raw_parts_mut(data_ptr, header.size))
              .ok_or(AllocError);
          }
        }
        last = current;
        current = data.next;
      }

      // println!("{}", unsafe { inner.base.read() });
    }
    Err(AllocError)
  }

  unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout)
  {
    unsafe {
      let p_meta = ptr.byte_sub(size_of::<Metadata>()).as_ptr() as *mut Metadata;
      let meta = *p_meta;

      if let Ok(mut inner) = self.inner.lock()
      {
        let new_header = Metadata::new(meta.size + meta.pad_bytes)
          .with_padding(0)
          .with_next(inner.base);
        new_header.write_to_ptr(meta.get_base(p_meta) as *mut Metadata);
        inner.base = new_header.get_base(p_meta) as *mut Metadata;
        println!("{}", inner.base.read());
      }
    }
  }
}

unsafe impl GlobalAlloc for MetaAlloc
{
  unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8
  {
    if layout.size() == 0
    {
      return null_mut();
    }
    self
      .allocate(layout)
      .map(|x| x.as_ptr().as_mut_ptr())
      .unwrap_or(null_mut())
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout)
  {
    if let Some(nn) = NonNull::new(ptr)
    {
      unsafe { self.deallocate(nn, layout) };
    }
  }
}

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
