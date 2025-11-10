use core::marker::PhantomData;
use core::ptr::NonNull;

pub type Link<T> = Option<NonNull<Node<T>>>;

pub struct Node<T>
{
  pub(self) front: Link<T>,
  pub(self) back: Link<T>,
  pub(self) elem: T,
}

pub struct LinkedDeque<T>
{
  front: Link<T>,
  back: Link<T>,
  len: usize,
  // _lifetime: PhantomData<&'aT>,
}

impl<T> Node<T>
{
  pub fn new(elem: T) -> Self
  {
    Self {
      elem,
      front: None,
      back: None,
    }
  }

  pub fn remove(&mut self)
  {
    unsafe {
      if let Some(front) = self.front
      {
        (*front.as_ptr()).back = self.back;
      }

      if let Some(back) = self.back
      {
        (*back.as_ptr()).front = self.front
      }
    }
    self.front = None;
    self.back = None;
  }

  pub fn elem(&self) -> &T
  {
    &self.elem
  }
  pub fn elem_mut(&mut self) -> &mut T
  {
    &mut self.elem
  }
}

impl<T> Default for LinkedDeque<T>
{
  fn default() -> Self
  {
    Self {
      front: Default::default(),
      back: Default::default(),
      len: Default::default(),
      // _lifetime: PhantomData,
    }
  }
}

impl<T> LinkedDeque<T>
{
  pub fn push_front(&mut self, new: NonNull<Node<T>>)
  {
    unsafe {
      if let Some(old) = self.front
      {
        (*old.as_ptr()).front = Some(new);
        (*new.as_ptr()).back = Some(old);
      }
      else
      {
        self.back = Some(new);
      }
      self.front = Some(new);
      self.len += 1;
    }
  }

  pub fn push_back(&mut self, new: NonNull<Node<T>>)
  {
    unsafe {
      if let Some(old) = self.back
      {
        (*old.as_ptr()).back = Some(new);
        (*new.as_ptr()).front = Some(old);
      }
      else
      {
        self.front = Some(new);
      }
      self.back = Some(new);
      self.len += 1;
    }
  }

  pub fn pop_front(&mut self) -> Link<T>
  {
    unsafe {
      self.front.map(|node| {
        self.front = (*node.as_ptr()).back;
        if let Some(new) = self.front
        {
          (*new.as_ptr()).front = None;
        }
        else
        {
          self.back = None;
        }
        self.len -= 1;
        node
      })
    }
  }

  pub fn pop_back(&mut self) -> Link<T>
  {
    unsafe {
      self.front.map(|node| {
        self.back = (*node.as_ptr()).front;
        if let Some(new) = self.back
        {
          (*new.as_ptr()).back = None;
        }
        else
        {
          self.front = None;
        }
        self.len -= 1;
        node
      })
    }
  }

  pub fn cursor_mut(&mut self) -> CursorMut<T>
  {
    CursorMut {
      current: None,
      list: self,
      index: None,
    }
  }

  pub fn len(&self) -> usize
  {
    self.len
  }

  pub fn empty(&self) -> bool
  {
    self.len == 0
  }
}

impl<T> Drop for LinkedDeque<T>
{
  fn drop(&mut self)
  {
    assert_eq!(
      self.len, 0,
      "This list is not responsible for deallocation of the memory it contains, please empty it before it gets dropped"
    );
  }
}

pub struct CursorMut<'a, T>
{
  current: Link<T>,
  list: &'a mut LinkedDeque<T>,
  index: Option<usize>,
}

impl<'a, T> CursorMut<'a, T>
{
  pub fn index(&self) -> Option<usize>
  {
    self.index
  }

  pub fn move_next(&mut self)
  {
    if let Some(cur) = self.current
    {
      unsafe {
        self.current = (*cur.as_ptr()).back;
        if self.current.is_some()
        {
          *self.index.as_mut().unwrap() += 1;
        }
        else
        {
          self.index = None
        }
      }
    }
    else if !self.list.empty()
    {
      self.current = self.list.front;
      self.index = Some(0);
    }
  }

  pub fn move_prev(&mut self)
  {
    if let Some(cur) = self.current
    {
      unsafe {
        self.current = (*cur.as_ptr()).front;
        if self.current.is_some()
        {
          *self.index.as_mut().unwrap() -= 1;
        }
        else
        {
          self.index = None
        }
      }
    }
    else if !self.list.empty()
    {
      self.current = self.list.back;
      self.index = Some(self.list.len() - 1);
    }
  }

  pub fn current(&mut self) -> Option<&mut T>
  {
    unsafe { self.current.map(|node| &mut (*node.as_ptr()).elem) }
  }

  pub fn peek_next(&mut self) -> Option<&mut T>
  {
    unsafe {
      self
        .current
        .and_then(|node| (*node.as_ptr()).back)
        .or_else(|| self.list.front)
        .map(|node| &mut (*node.as_ptr()).elem)
    }
  }

  pub fn peek_previous(&mut self) -> Option<&mut T>
  {
    unsafe {
      self
        .current
        .and_then(|node| (*node.as_ptr()).front)
        .or_else(|| self.list.back)
        .map(|node| &mut (*node.as_ptr()).elem)
    }
  }

  pub fn split_before(&mut self) -> LinkedDeque<T>
  {
    if let Some(cur) = self.current
    {
      unsafe {
        let old_len = self.list.len();
        let old_idx = self.index.unwrap();
        let prev = (*cur.as_ptr()).front;

        let new_len = old_len - old_idx;
        let new_front = self.current;
        let new_back = self.list.back;
        let new_idx = Some(0);

        let out_len = old_len - new_len;
        let out_front = self.list.front;
        let out_back = prev;

        if let Some(prev) = prev
        {
          (*cur.as_ptr()).front = None;
          (*prev.as_ptr()).back = None;
        }

        self.list.len = new_len;
        self.list.front = new_front;
        self.list.back = new_back;
        self.index = new_idx;

        LinkedDeque {
          front: out_front,
          back: out_back,
          len: out_len,
          // _lifetime: PhantomData,
        }
      }
    }
    else
    {
      core::mem::replace(self.list, Default::default())
    }
  }

  pub fn split_after(&mut self) -> LinkedDeque<T>
  {
    // We have this:
    //
    //     list.front -> A <-> B <-> C <-> D <- list.back
    //                         ^
    //                        cur
    //
    //
    // And we want to produce this:
    //
    //     list.front -> A <-> B <- list.back
    //                         ^
    //                        cur
    //
    //
    //    return.front -> C <-> D <- return.back
    //
    if let Some(cur) = self.current
    {
      // We are pointing at a real element, so the list is non-empty.
      unsafe {
        // Current state
        let old_len = self.list.len;
        let old_idx = self.index.unwrap();
        let next = (*cur.as_ptr()).back;

        // What self will become
        let new_len = old_idx + 1;
        let new_back = self.current;
        let new_front = self.list.front;
        let new_idx = Some(old_idx);

        // What the output will become
        let output_len = old_len - new_len;
        let output_front = next;
        let output_back = self.list.back;

        // Break the links between cur and next
        if let Some(next) = next
        {
          (*cur.as_ptr()).back = None;
          (*next.as_ptr()).front = None;
        }

        // Produce the result:
        self.list.len = new_len;
        self.list.front = new_front;
        self.list.back = new_back;
        self.index = new_idx;

        LinkedDeque {
          front: output_front,
          back: output_back,
          len: output_len,
          // _lifetime: PhantomData,
        }
      }
    }
    else
    {
      // We're at the ghost, just replace our list with an empty one.
      // No other state needs to be changed.
      std::mem::replace(self.list, Default::default())
    }
  }

  pub fn remove(&mut self) -> Link<T>
  {
    let current = self.current.clone();
    self.move_next();
    if let Some(cur) = current
    {
      unsafe {
        let n = &mut (*cur.as_ptr());

        match (n.front, n.back)
        {
          (None, Some(next)) => (*next.as_ptr()).front = None,
          (Some(prev), None) => (*prev.as_ptr()).back = None,
          (Some(prev), Some(next)) =>
          {
            (*prev.as_ptr()).back = Some(next);
            (*next.as_ptr()).front = Some(prev);
          }
          _ => (),
        }

        n.front = None;
        n.back = None;

        self.list.len -= 1;
      }
    }
    current
  }

  pub fn insert_before(&mut self, node: NonNull<Node<T>>)
  {
    if let Some(cur) = self.current
    {
      unsafe {
        let n = &mut (*cur.as_ptr());
        (*node.as_ptr()).back = Some(cur);
        if let Some(x) = n.front
        {
          (*x.as_ptr()).back = Some(node);
        }
        self.index = self.index.map(|x| x + 1);
        self.list.len += 1;
      }
    }
    else
    {
      // on ghost!
      self.list.push_front(node);
      self.move_next();
    }
  }
}
