use core::alloc::Layout;
use core::iter::{DoubleEndedIterator, Iterator};
use core::marker::PhantomData;
use core::ptr::null_mut;
use core::slice;

#[derive(Clone)]
#[repr(packed)]
pub(crate) struct MetaData
{
  layout: Layout,
  padding: usize,
  pub next: *mut MetaData,
  pub previous: *mut MetaData,
  allocated: bool,
}

pub(crate) struct ManagedBytes<'a>
{
  base: *mut u8,
  layout: Layout,
  _lifetime: PhantomData<&'a mut [u8]>,
}

pub(crate) struct MetaIter<'a>
{
  current: *mut MetaData,
  _lifetime: PhantomData<&'a MetaData>,
}

impl<'a> Iterator for MetaIter<'a>
{
  type Item = &'a mut MetaData;

  fn next(&mut self) -> Option<Self::Item>
  {
    if let Some(actual) = unsafe { self.current.as_ref() }
    {
      self.current = actual.next;
      actual.get_next_mut()
    }
    else
    {
      None
    }
  }
}

impl<'a> DoubleEndedIterator for MetaIter<'a>
{
  fn next_back(&mut self) -> Option<Self::Item>
  {
    if let Some(actual) = unsafe { self.current.as_ref() }
    {
      self.current = actual.previous;
      actual.get_previous_mut()
    }
    else
    {
      None
    }
  }
}

impl MetaData
{
  pub fn as_bytes<'a>(&'a self) -> &'a [u8]
  {
    unsafe { core::slice::from_raw_parts(self as *const MetaData as *const u8, size_of::<Self>()) }
  }

  pub fn new(layout: Layout, padding: usize) -> Self
  {
    Self {
      layout,
      padding,
      next: null_mut(),
      previous: null_mut(),
      allocated: false,
    }
  }

  pub fn set_next(&mut self, next: *mut Self)
  {
    self.next = next;
  }

  pub fn get_next<'a>(&'a self) -> Option<&'a Self>
  {
    unsafe { self.next.as_ref() }
  }

  pub fn get_next_mut<'a>(&'a self) -> Option<&'a mut Self>
  {
    unsafe { self.next.as_mut() }
  }

  pub fn set_previous(&mut self, prev: *mut Self)
  {
    self.previous = prev;
  }

  pub fn get_previous<'a>(&'a self) -> Option<&'a Self>
  {
    unsafe { self.previous.as_ref() }
  }

  pub fn get_previous_mut<'a>(&'a self) -> Option<&'a mut Self>
  {
    unsafe { self.previous.as_mut() }
  }

  pub fn get_block_size(&self) -> usize
  {
    let layout = self.layout;
    layout.size() + self.padding + size_of::<Self>()
  }

  pub fn is_allocated(&self) -> bool
  {
    self.allocated
  }

  pub fn set_allocated(&mut self, allocated: bool)
  {
    self.allocated = allocated;
  }
}

impl<'a> ManagedBytes<'a>
{
  pub fn new(base: *mut u8, layout: Layout) -> Self
  {
    Self {
      base,
      layout,
      _lifetime: Default::default(),
    }
  }

  pub fn get_data_offset(&self) -> usize
  {
    unsafe {
      self
        .base
        .add(size_of::<MetaData>())
        .align_offset(self.layout.align())
    }
  }
  pub fn get_meta_offset(&self) -> usize
  {
    self.get_data_offset() - size_of::<MetaData>()
  }

  fn calc_total_size(&self) -> usize
  {
    let padding_offset = self.get_meta_offset();
    self.layout.size() + padding_offset + size_of::<MetaData>()
  }

  pub fn get_data(&mut self) -> &'a mut [u8]
  {
    unsafe { slice::from_raw_parts_mut(self.base, self.calc_total_size()) }
  }

  pub fn split_with_layout(mut self, layout: Layout) -> (Self, Self)
  {
    let slice = self.get_data();
    let original_size = slice.len();

    let mut rhs = Self {
      base: null_mut(),
      layout,
      _lifetime: Default::default(),
    };

    let new_size = original_size - rhs.calc_total_size();

    let (lslice, rslice) = slice.split_at_mut(new_size);

    // should check this lmao
    self.layout = unsafe { Layout::from_size_align_unchecked(new_size, self.layout.align()) };

    self.base = lslice.as_mut_ptr();
    rhs.base = rslice.as_mut_ptr();

    (self, rhs)
  }
}
