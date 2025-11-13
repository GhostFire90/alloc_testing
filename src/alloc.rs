use crate::deque_list::{Link, LinkedDeque, Node};

use core::alloc::{Allocator, GlobalAlloc, Layout};
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
    let meta_location = NonNull::new(
      nnptr.add(nnptr.align_offset(meta.layout.align())).as_ptr() as *mut Node<MetaData>
    )
    .unwrap();
    *(meta_location.as_ptr()) = Node::new(meta);
    meta_location
  }
}

fn meta_write(meta: MetaData) -> NonNull<Node<MetaData>>
{
  unsafe {
    let base_meta = NonNull::new(
      meta
        .base
        .add(meta.base.align_offset(meta.layout.align()))
        .as_ptr() as *mut Node<MetaData>,
    )
    .unwrap();
    (*base_meta.as_ptr()) = Node::new(meta);
    base_meta
  }
}

fn node_split(
  node: NonNull<Node<MetaData>>,
  layout: Layout,
) -> (NonNull<Node<MetaData>>, Option<NonNull<Node<MetaData>>>)
{
  unsafe {
    let old_meta = (*node.as_ptr()).elem();
    let total_size = old_meta.total_size();
    let mut lhs = MetaData::new(old_meta.base, layout);

    let lhs_size = lhs.total_size();

    let remaining_size = total_size - lhs_size;

    let rhs_base = lhs.base.add(lhs_size);
    let mut o_rhs = None;

    let meta_total_size = NODE_SIZE + rhs_base.align_offset(NODE_ALIGN);

    if remaining_size != 0
    {
      if remaining_size > meta_total_size
      {
        let size = remaining_size - (NODE_SIZE + rhs_base.align_offset(NODE_ALIGN));
        o_rhs = Some(meta_write(MetaData::new(
          rhs_base,
          Layout::from_size_align(size, NODE_ALIGN).unwrap(),
        )))
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

fn node_merge(lhs: NonNull<Node<MetaData>>, rhs: NonNull<Node<MetaData>>)
{
  unsafe {
    let l = (*lhs.as_ptr()).elem_mut();
    let r = (*rhs.as_ptr()).elem();

    if l.base.add(l.total_size()) == r.base
    {
      l.layout =
        Layout::from_size_align(l.layout.size() + r.total_size(), l.layout.align()).unwrap()
    }
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

struct MetaAllocInner
{
  list: LinkedDeque<MetaData>,
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
      tex: Mutex::new(MetaAllocInner {
        list: LinkedDeque::new(),
      }),
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
      let node = raw_to_new_node(pg, PAGE_LAYOUT);
      unsafe {
        self.dealloc(node_to_data_ptr(node), PAGE_LAYOUT);
      };
      true
    }
  }

  unsafe fn alloc(&mut self, layout: Layout) -> *mut u8
  {
    dbg!(layout.size());
    dbg!(self.list.len());
    dbg!(
      self
        .list
        .peek_front()
        .map(|x| unsafe { (*x.as_ptr()).clone() })
    );
    if self.list.empty()
    {
      if !unsafe { self.try_add_page() }
      {
        return core::ptr::null_mut();
      }
    }

    let mut cursor = self.list.cursor_mut();
    cursor.move_next();
    while let Some(current) = cursor.current()
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

    dbg!(FAKE_HTOP.load(std::sync::atomic::Ordering::Relaxed));
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
      while let Some(current) = cursor.current()
      {
        if *current > *(*node.as_ptr()).elem()
        {
          cursor.insert_before(node);
          cursor.move_prev();

          let o_prev = cursor.prev_link();
          let o_next = cursor.next_link();
          match (o_prev, o_next)
          {
            (None, None) =>
            {}
            (None, Some(n)) =>
            {
              node_merge(node, n);
              cursor.move_next();
              cursor.remove();
            }
            (Some(p), None) =>
            {
              node_merge(p, node);
              cursor.remove();
            }
            (Some(p), Some(n)) =>
            {
              node_merge(p, node);
              node_merge(p, n);
              cursor.remove();
              cursor.remove();
            }
          }

          return;
        }
        cursor.move_next();
      }

      self.list.push_back(node);
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
  fn pad_amount(&self, align: usize) -> usize
  {
    self.base.align_offset(align) - self.base.align_offset(self.layout.align())
  }
  pub fn usable_size(&self) -> usize
  {
    self.layout.size() + (self.layout.align() - NODE_ALIGN)
  }

  pub fn total_size(&self) -> usize
  {
    let offs = self.base.align_offset(self.layout.align());
    self.layout.size() + offs + NODE_SIZE
  }

  pub fn check_compatible(&self, lay: &Layout) -> bool
  {
    if lay.align() > self.layout.align()
    {
      (self.usable_size() - self.pad_amount(lay.align())) >= lay.size()
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
    let offs = ret.pad_amount(layout.align());
    ret.layout = unsafe {
      Layout::from_size_align_unchecked(ret.layout.size() - offs, layout.align().max(NODE_ALIGN))
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
  const ALLOC_COUNT: usize = 10000;

  use core::alloc::{Allocator, Layout};
  use core::ptr::NonNull;
  use std::alloc::GlobalAlloc;
  use std::ptr::null_mut;

  use crate::MetaAlloc;
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
      stored.into_iter().enumerate().for_each(|(i, x)| unsafe {
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
      stored.into_iter().rev().for_each(|x| unsafe {
        myalloc.dealloc(x, LAY);
      });
    }
  }
}
