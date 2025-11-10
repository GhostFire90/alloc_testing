use crate::deque_list::{Link, LinkedDeque, Node};

use core::alloc::{Allocator, GlobalAlloc, Layout};
use core::num::NonZero;
use core::ptr::NonNull;
use std::sync::Mutex;

const PAGE_SIZE: usize = 4096;
const NODE_SIZE: usize = size_of::<Node<MetaData>>();
const NODE_ALIGN: usize = align_of::<Node<MetaData>>();

#[derive(Clone)]
pub struct MetaData
{
  pub base: NonZero<usize>,
  pub layout: Layout,
  pub flags: u32,
}

const PAGE_LAYOUT: Layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };

fn get_page<'a>() -> Option<NonNull<u8>>
{
  unsafe { NonNull::new(std::alloc::System.alloc(PAGE_LAYOUT)) }
}

struct MetaAllocInner
{
  list: LinkedDeque<MetaData>,
}

pub struct MetaAlloc
{
  inner: Mutex<MetaAllocInner>,
}

unsafe fn write_metadata(ptr: Option<NonNull<u8>>, layout: Layout, meta: MetaData)
-> Link<MetaData>
{
  let align = core::cmp::max(layout.align(), align_of::<MetaData>());
  if let Some(x) = ptr.map(|x| unsafe { x.byte_add(x.align_offset(align)) })
  {
    unsafe {
      *(x.as_ptr() as *mut Node<MetaData>) = Node::new(meta);
    }
    NonNull::new(x.as_ptr() as *mut Node<MetaData>)
  }
  else
  {
    None
  }
}

impl MetaAllocInner
{
  pub fn alloc(&mut self, layout: Layout) -> *mut u8
  {
    let align = layout.align().max(NODE_ALIGN);
  }

  pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout)
  {
    unsafe {
      let pnode =
        NonNull::new_unchecked(ptr.byte_sub(size_of::<Node<MetaData>>()) as *mut Node<MetaData>);
      let node = &(*pnode.as_ptr());

      if node.elem().layout != layout
      {
        panic!("Ptr metadata corrupt, layout not equal");
      }

      let given_align = node.elem().layout.align().max(align_of::<Node<MetaData>>());
      let total_block_size = node.elem().layout.size() + given_align + size_of::<Node<MetaData>>();

      let mut cursor = self.list.cursor_mut();

      if cursor.current().is_none()
      {
        self.list.push_front(pnode);
        return;
      }

      while let Some(current) = cursor.current()
      {
        // keep sorted
        if current.base > node.elem().base
        {
          let align = current.layout.align().max(align_of::<Node<MetaData>>());
          if current.base.get() + current.layout.size() + align == node.elem().base.get()
          {
            // this means they are neighbors!
            // absorb the block and drop the ptr
            current.layout =
              Layout::from_size_align_unchecked(current.layout.size() + total_block_size, align);
          }
          else
          {
            let new_meta = MetaData {
              base: pnode.byte_sub(given_align).addr(),
              layout: Layout::from_size_align_unchecked(
                total_block_size - NODE_ALIGN - NODE_SIZE,
                NODE_ALIGN,
              ),
              flags: 0,
            };
            let link = write_metadata(
              NonNull::new(new_meta.base.get() as *mut u8),
              new_meta.layout,
              new_meta,
            );
            cursor.insert_before(link.unwrap());
          }
          return;
        }
        cursor.move_next();
      }
    }
  }

  pub fn add_page(&mut self)
  {
    let ptr = get_page().unwrap();
    self.list.push_back(unsafe {
      write_metadata(
        Some(ptr.clone()),
        PAGE_LAYOUT,
        MetaData {
          base: ptr.addr(),
          layout: PAGE_LAYOUT,
          flags: 0,
        },
      )
      .unwrap()
    });
  }
}

unsafe impl GlobalAlloc for MetaAlloc
{
  unsafe fn alloc(&self, layout: Layout) -> *mut u8
  {
    todo!()
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout)
  {
    todo!()
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
