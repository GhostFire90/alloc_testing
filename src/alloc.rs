use raw_list::{Link, List, Node};

use core::alloc::{GlobalAlloc, Layout};
use core::num::NonZero;
use core::ptr::NonNull;
use std::alloc::System;
use std::cell::UnsafeCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicUsize};

const PAGE_SIZE: usize = 4096;
const NODE_SIZE: usize = size_of::<Node<MetaData>>();
const NODE_ALIGN: usize = align_of::<Node<MetaData>>();

#[derive(Debug, Clone, PartialEq)]
pub struct MetaData
{
  pub base: NonNull<u8>,
  pub layout: Layout,
}

const PAGE_LAYOUT: Layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };

static FAKE_HEAP_SIZE: usize = PAGE_SIZE * 1024 * 1024;
static FAKE_HEAP: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static FAKE_HTOP: AtomicUsize = AtomicUsize::new(0);

fn get_page() -> *mut u8
{
  if FAKE_HEAP
    .load(std::sync::atomic::Ordering::Acquire)
    .is_null()
  {
    FAKE_HEAP.store(
      unsafe { System.alloc(Layout::from_size_align(FAKE_HEAP_SIZE, PAGE_SIZE).unwrap()) },
      std::sync::atomic::Ordering::Release,
    );
  }

  if FAKE_HTOP.load(std::sync::atomic::Ordering::Relaxed) >= FAKE_HEAP_SIZE
  {
    return core::ptr::null_mut();
  }
  unsafe {
    let ptr = FAKE_HEAP
      .load(std::sync::atomic::Ordering::Relaxed)
      .add(FAKE_HTOP.load(std::sync::atomic::Ordering::Relaxed)) as *mut u8;
    FAKE_HTOP.fetch_add(PAGE_SIZE, std::sync::atomic::Ordering::Relaxed);
    ptr
  }
}

fn raw_to_new_node(ptr: *mut u8, layout: Layout) -> NonNull<Node<MetaData>>
{
  unsafe {
    let nnptr = NonNull::new(ptr).unwrap();
    let meta = MetaData::new(
      nnptr,
      Layout::from_size_align_unchecked(layout.size() - NODE_SIZE, layout.align()),
    );

    let mut data_ptr = nnptr.byte_add(NODE_SIZE);
    data_ptr = data_ptr.byte_add(data_ptr.align_offset(meta.layout.align()));

    let meta_location = raw_to_existing_node(data_ptr.as_ptr());
    *(meta_location.as_ptr()) = Node::new(meta);
    meta_location
  }
}

fn meta_write(meta: MetaData) -> NonNull<Node<MetaData>>
{
  unsafe {
    let mut data_ptr = meta.base.byte_add(NODE_SIZE);
    data_ptr = data_ptr.byte_add(data_ptr.align_offset(meta.layout.align()));
    let meta_location = raw_to_existing_node(data_ptr.as_ptr());
    (*meta_location.as_ptr()) = Node::new(meta);
    meta_location
  }
}

fn node_split(
  node: NonNull<Node<MetaData>>,
  layout: Layout,
) -> (NonNull<Node<MetaData>>, Option<NonNull<Node<MetaData>>>)
{
  unsafe {
    let old_meta = (*node.as_ptr()).elem();
    let align = layout.align().max(NODE_ALIGN);
    let needed_size = MetaData::pad_amount(old_meta.base, align) + NODE_SIZE + layout.size();
    let total_layout = Layout::from_size_align(needed_size, align).unwrap();

    let total_size = old_meta.total_size();
    let mut lhs = MetaData::new(old_meta.base, total_layout);

    let lhs_size = lhs.total_size();
    assert!(lhs_size == needed_size);

    let remaining_size = total_size - lhs_size;

    let rhs_base = lhs.base.add(lhs_size);
    let mut o_rhs = None;

    let meta_total_size = MetaData::optimal_padding(rhs_base) + NODE_SIZE;

    if remaining_size != 0
    {
      if remaining_size > meta_total_size
      {
        let rhs_meta = MetaData::new_blank(rhs_base, remaining_size);
        o_rhs = Some(meta_write(rhs_meta));
      }
      else
      {
        let excess = meta_total_size - remaining_size;
        lhs.layout =
          Layout::from_size_align_unchecked(lhs.layout.size() + excess, lhs.layout.align());
      }
    }
    (meta_write(lhs), o_rhs)
  }
}

fn raw_to_existing_node(ptr: *mut u8) -> NonNull<Node<MetaData>>
{
  unsafe { NonNull::new(ptr.byte_sub(NODE_SIZE) as *mut Node<MetaData>).unwrap() }
}

fn node_to_data_ptr(node: NonNull<Node<MetaData>>) -> *mut u8
{
  unsafe { node.byte_add(NODE_SIZE).as_ptr() as *mut u8 }
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
    // dbg!(layout.size());
    // dbg!(self.list.len());
    // dbg!(
    //   self
    //     .list
    //     .peek_front()
    //     .map(|x| unsafe { (*x.as_ptr()).clone() })
    // );
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
        // PAST THIS POINT ACCESSES TO CURRENT ARE F U C K E D
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

    // dbg!(FAKE_HTOP.load(std::sync::atomic::Ordering::Relaxed));
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
  fn pad_amount(base: NonNull<u8>, align: usize) -> usize
  {
    unsafe {
      let mut data_ptr = base.byte_add(NODE_SIZE);
      data_ptr = data_ptr.byte_add(data_ptr.align_offset(align));
      data_ptr.byte_sub(NODE_SIZE).offset_from_unsigned(base)
    }
  }

  // padding if align was <= NODE_ALIGN
  pub fn optimal_padding(base: NonNull<u8>) -> usize
  {
    unsafe {
      let mut data_ptr = base.byte_add(NODE_SIZE);
      data_ptr = data_ptr.byte_add(data_ptr.align_offset(NODE_ALIGN));
      data_ptr.byte_sub(NODE_SIZE).offset_from_unsigned(base)
    }
  }

  pub fn usable_size(&self) -> usize
  {
    let node_padding = Self::optimal_padding(self.base);
    let current_padding = Self::pad_amount(self.base, self.layout.align());

    self.layout.size() + (current_padding - node_padding)
  }

  pub fn total_size(&self) -> usize
  {
    Self::pad_amount(self.base, self.layout.align()) + NODE_SIZE + self.layout.size()
  }

  pub fn check_compatible(&self, lay: &Layout) -> bool
  {
    if lay.align() > self.layout.align()
    {
      self
        .usable_size()
        .checked_sub(Self::pad_amount(self.base, lay.align()))
        .map_or(false, |x| x >= lay.size())
    }
    else
    {
      self.usable_size() >= lay.size()
    }
  }
  // Creates a new metadata with the given base, it will do max(NODE_ALIGN, layout.align()) as well as adjust the size by the difference of that and NODE_SIZE
  pub fn new(base: NonNull<u8>, layout: Layout) -> Self
  {
    let mut ret = Self { base, layout };
    let align = layout.align().max(NODE_ALIGN);

    let offs = Self::pad_amount(ret.base, align);
    ret.layout = Layout::from_size_align(ret.layout.size() - offs - NODE_SIZE, align).unwrap();
    ret
  }
  pub fn new_blank(base: NonNull<u8>, size: usize) -> Self
  {
    let padding = Self::optimal_padding(base);
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
