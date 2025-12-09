use raw_list::{Link, List, Node};

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use std::alloc::System;
use std::sync::Mutex;

const PAGE_SIZE: usize = 4096;

// NODE_ALIGN*5
const NODE_SIZE: usize = size_of::<Node<MetaData>>();
// NODE_ALIGN = 8
const NODE_ALIGN: usize = align_of::<Node<MetaData>>();

// Time to steal from raw_rc
//
// - Node<MetaData> is a link in a doubly linked list containing allocation metadata
// - The alignment of the allocation is `align_of::<Node<MetaData>>().max(layout.align())`.
// - The value is stored at offset `size_of::<Node<MetaData>>().next_multiple_of(layout.align)`.
// - The size of the allocation is
//   `size_of::<Node<MetaData>>().next_multiple_of(layout.align()) + layout.size()`.
// - The `Node<MetaData>` object is stored at offset
//   `size_of::<Node<MetaData>>().next_multiple_of(layout.align()) - size_of::<Node<MetaData>>()`.
//
// The following table shows the order and size of each component in a reference-counted allocation
// of a `T` value:
//
// | Component   | Size                                                                                                                                                              |
// | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
// | Padding     | `basePtr.align_offset(align_of<Node<MetaData>>().max(layout.align())) size_of::<Node<MetaData>>().next_multiple_of(layout.align()) - size_of::<Node<MetaData>>()` |
// | `RefCounts` | `size_of::<Node<MetaData>>()`                                                                                                                                     |
// | `UserData`  | `layout.size`                                                                                                                                                     |
//

#[derive(Debug, Clone, PartialEq)]
pub struct MetaData
{
  // original base ptr, pre alignment
  pub base: NonNull<u8>,

  // requested layout, no modifications
  pub layout: Layout,
}

const PAGE_LAYOUT: Layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };

static FAKE_HEAP_SIZE: usize = PAGE_SIZE * 1024 * 1024;

struct FakeHeap
{
  base: *mut u8,
  current_top: usize,
}
// only using when wrapped in a mutex
unsafe impl Send for FakeHeap {}
impl Drop for FakeHeap
{
  fn drop(&mut self)
  {
    unsafe {
      System.dealloc(
        self.base,
        Layout::from_size_align(FAKE_HEAP_SIZE, PAGE_SIZE).unwrap(),
      );
    }
  }
}

static FAKE_HEAP: Mutex<FakeHeap> = Mutex::new(FakeHeap {
  current_top: 0,
  base: core::ptr::null_mut(),
});

fn get_page() -> *mut u8
{
  let mut guard = FAKE_HEAP.lock().expect("GET PAGE FAKE HEAP POISON");
  if guard.base.is_null()
  {
    guard.base =
      unsafe { System.alloc(Layout::from_size_align(FAKE_HEAP_SIZE, PAGE_SIZE).unwrap()) };
  }

  if guard.current_top >= FAKE_HEAP_SIZE
  {
    return core::ptr::null_mut();
  }

  unsafe {
    let ptr = guard.base.add(guard.current_top);
    guard.current_top += PAGE_SIZE;
    ptr
  }
}

fn meta_write(meta: MetaData) -> NonNull<Node<MetaData>>
{
  unsafe {
    let meta_ptr = meta.meta_location();
    (*meta_ptr.as_ptr()) = Node::new(meta);
    meta_ptr
  }
}

fn node_split(
  node: NonNull<Node<MetaData>>,
  layout: Layout,
) -> (NonNull<Node<MetaData>>, Option<NonNull<Node<MetaData>>>)
{
  unsafe {
    let original = (*node.as_ptr()).elem().clone();
    let block_size = original.total_size();

    let mut lhs = MetaData::new(original.base, layout);
    let lhs_size = lhs.total_size();

    let remaining_size = block_size - lhs_size;
    let rhs_ptr = lhs.base.byte_add(lhs_size);
    let required_size = MetaData::default_meta_offset(rhs_ptr) + NODE_SIZE;

    if remaining_size > required_size
    {
      let rhs = MetaData::new(
        rhs_ptr,
        Layout::from_size_align(remaining_size - required_size, NODE_ALIGN).unwrap(),
      );

      (meta_write(lhs), Some(meta_write(rhs)))
    }
    else
    {
      lhs.layout =
        Layout::from_size_align(lhs.layout.size() + remaining_size, lhs.layout.align()).unwrap();
      (meta_write(lhs), None)
    }
  }
}

fn raw_to_existing_node(ptr: *mut u8) -> NonNull<Node<MetaData>>
{
  unsafe {
    NonNull::new(
      FAKE_HEAP
        .lock()
        .expect("FAKE HEAP POISON RAW TO EXISTING")
        .base
        .with_addr(ptr.byte_sub(NODE_SIZE).addr()) as *mut Node<MetaData>,
    )
    .unwrap()
  }
}

fn node_to_data_ptr(node: NonNull<Node<MetaData>>) -> *mut u8
{
  unsafe { (*node.as_ptr()).elem().data_location().as_ptr() }
}

fn merge_right(link: Link<MetaData>) -> bool
{
  unsafe {
    if let Some(p_node) = link
    {
      let node = &mut (*p_node.as_ptr());

      if let Some(p_right) = node.next_node()
      {
        let right = &(*p_right.as_ptr());
        let right_meta = right.elem();

        let node_meta = node.elem_mut();
        if node_meta.base.byte_add(node_meta.total_size()) == right_meta.base
        {
          node_meta.layout = Layout::from_size_align(
            node_meta.layout.size() + right_meta.total_size(),
            node_meta.layout.align(),
          )
          .unwrap();
          true
        }
        else
        {
          false
        }
      }
      else
      {
        false
      }
    }
    else
    {
      false
    }
  }
}

struct MetaAllocInner
{
  list: List<MetaData>,
}

pub struct MetaAlloc
{
  tex: Mutex<MetaAllocInner>,
}
unsafe impl Send for MetaAlloc {}
unsafe impl Sync for MetaAlloc {}

impl MetaAlloc
{
  pub const fn new() -> Self
  {
    Self {
      tex: Mutex::new(MetaAllocInner { list: List::new() }),
    }
  }
}

impl MetaAllocInner
{
  unsafe fn try_add_page(&mut self) -> bool
  {
    let pg = get_page();
    if pg.is_null()
    {
      false
    }
    else
    {
      let meta = MetaData::new_blank(NonNull::new(pg).unwrap(), PAGE_SIZE);
      let node = meta_write(meta);
      unsafe {
        self.dealloc(node_to_data_ptr(node), PAGE_LAYOUT);
      };
      true
    }
  }

  unsafe fn alloc(&mut self, layout: Layout) -> *mut u8
  {
    if self.list.empty()
    {
      if !unsafe { self.try_add_page() }
      {
        return core::ptr::null_mut();
      }
    }

    let mut cursor = self.list.cursor_mut();
    cursor.move_next();
    while let Some(current) = cursor.current_value()
    {
      if current.check_compatible(&layout)
      {
        let node = cursor.remove().unwrap();

        let (ret_node, remaining) = node_split(node, layout);
        if let Some(rem) = remaining
        {
          unsafe { self.dealloc(node_to_data_ptr(rem), (*rem.as_ptr()).elem().layout) };
        }
        return node_to_data_ptr(ret_node);
      }
      cursor.move_next();
    }

    if !unsafe { self.try_add_page() }
    {
      core::ptr::null_mut()
    }
    else
    {
      unsafe { self.alloc(layout) }
    }
  }

  unsafe fn dealloc(&mut self, ptr: *mut u8, _layout: Layout)
  {
    let node = raw_to_existing_node(ptr);
    if self.list.empty()
    {
      self.list.push_front(node);
      return;
    }

    unsafe {
      let mut cursor = self.list.cursor_mut();
      cursor.move_next();
      while let Some(current) = cursor.current_value()
      {
        if *current > *(*node.as_ptr()).elem()
        {
          cursor.insert_before(node);
          cursor.move_prev();

          if merge_right(cursor.current_link())
          {
            cursor.move_next();
            cursor.remove();
            cursor.move_prev();
          }

          cursor.move_prev();
          if merge_right(cursor.current_link())
          {
            cursor.move_next();
            cursor.remove();
          }

          return;
        }
        cursor.move_next();
      }

      self.list.push_back(node);
      let mut end_cursor = self.list.cursor_mut();
      end_cursor.move_prev();
      end_cursor.move_prev();
      if merge_right(end_cursor.current_link())
      {
        self.list.pop_back();
      }
    }
  }
}

unsafe impl GlobalAlloc for MetaAlloc
{
  unsafe fn alloc(&self, layout: Layout) -> *mut u8
  {
    unsafe {
      self
        .tex
        .lock()
        .expect("Meta alloc tex poison alloc")
        .alloc(layout)
    }
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout)
  {
    unsafe {
      self
        .tex
        .lock()
        .expect("Meta alloc tex poison dealloc")
        .dealloc(ptr, layout)
    };
  }
}

impl MetaData
{
  pub fn data_location(&self) -> NonNull<u8>
  {
    // base + padding + metadata size
    unsafe { self.base.byte_add(self.extra_size()) }
  }

  pub fn meta_location(&self) -> NonNull<Node<MetaData>>
  {
    // data_location - node size
    unsafe { self.data_location().byte_sub(NODE_SIZE).cast() }
  }

  pub fn extra_size(&self) -> usize
  {
    // bytes to align to `self.layout.align().max(NODE_ALIGN)` so the rest of the calculation is correct
    let align = self.base.align_offset(self.layout.align().max(NODE_ALIGN));

    NODE_SIZE.next_multiple_of(self.layout.align()) + align
  }

  pub fn default_meta_offset(base: NonNull<u8>) -> usize
  {
    let temp_meta = MetaData::new(
      base.clone(),
      Layout::from_size_align(0, NODE_ALIGN).unwrap(),
    );
    unsafe { temp_meta.meta_location().byte_offset_from_unsigned(base) }
  }

  pub fn usable_size(&self) -> usize
  {
    let node_padding = Self::default_meta_offset(self.base);
    let current_padding = unsafe { self.meta_location().byte_offset_from_unsigned(self.base) };

    self.layout.size() + (current_padding - node_padding)
  }

  pub fn total_size(&self) -> usize
  {
    self.layout.size() + self.extra_size()
  }

  pub fn check_compatible(&self, lay: &Layout) -> bool
  {
    if lay.align() > self.layout.align()
    {
      let new_meta = Self::new(self.base.clone(), lay.clone());

      self
        .total_size()
        .checked_sub(new_meta.extra_size())
        .map_or(false, |x| x >= lay.size())
    }
    else
    {
      self.usable_size() >= lay.size()
    }
  }

  pub fn new(base: NonNull<u8>, layout: Layout) -> Self
  {
    Self { base, layout }
  }

  // used to create a blank unallocated node with the correct sizing from a new page
  pub fn new_blank(base: NonNull<u8>, size: usize) -> Self
  {
    let padding = Self::default_meta_offset(base);
    let total_removed = padding + NODE_SIZE;
    let ret = Self {
      base,
      layout: Layout::from_size_align(size - total_removed, NODE_ALIGN).unwrap(),
    };

    ret
  }
}

impl PartialOrd for MetaData
{
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering>
  {
    self.base.partial_cmp(&other.base)
  }
}

#[cfg(test)]
mod meta_tests
{
  const ALLOC_COUNT: usize = 1000;

  use core::alloc::Layout;
  use std::alloc::GlobalAlloc;

  use crate::{MetaAlloc, alloc::PAGE_LAYOUT};
  const LAY: Layout = unsafe { Layout::from_size_align_unchecked(32, 16) };

  #[test]
  pub fn fifo_alloc()
  {
    unsafe {
      let myalloc: MetaAlloc = MetaAlloc::new();
      let mut stored: Vec<*mut u8> = Vec::with_capacity(ALLOC_COUNT);
      for i in 1..ALLOC_COUNT
      {
        let a = myalloc.alloc(Layout::from_size_align(i, 8).unwrap());
        assert!(!a.is_null());
        assert!(a.is_aligned_to(8));
        a.write_bytes(0xff, i);

        stored.push(a);
      }
      stored.into_iter().enumerate().for_each(|(i, x)| {
        myalloc.dealloc(x, Layout::from_size_align(i, 8).unwrap());
      });
    }
  }

  #[test]
  pub fn lifo_alloc()
  {
    unsafe {
      let myalloc: MetaAlloc = MetaAlloc::new();
      let mut stored = Vec::with_capacity(ALLOC_COUNT);
      for _ in 0..ALLOC_COUNT
      {
        let ptr = myalloc.alloc(LAY);
        assert!(!ptr.is_null());
        assert!(ptr.is_aligned_to(LAY.align()));
        stored.push(ptr);
      }
      stored.into_iter().rev().for_each(|x| {
        myalloc.dealloc(x, LAY);
      });
    }
  }

  #[test]
  pub fn page_alloc()
  {
    unsafe {
      let myalloc = MetaAlloc::new();
      let ptr = myalloc.alloc(PAGE_LAYOUT);
      assert!(!ptr.is_null());
      myalloc.dealloc(ptr, PAGE_LAYOUT);
    }
  }

  #[test]
  pub fn align_test() {}
}
