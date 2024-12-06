//! Idiomatic and high-level abstraction over [`gtk::ListView`].

#![allow(dead_code)]

// use super::{get_mut_value, Filter, OrdFn, RelmSelectionExt, TypedListItem};
// use gtk::{
//   gio, glib,
//   prelude::{
//     Cast, CastNone, FilterExt, IsA, ListItemExt, ListModelExt, ObjectExt,
//   },
// };
use std::cell::Ref;
use std::marker::PhantomData;
use std::{any::Any, cell::RefMut};

use gio::glib;
use gio::prelude::*;
use gtk::prelude::*;
use gtk4 as gtk;

fn get_value<T: 'static>(obj: &glib::Object) -> Ref<'_, T> {
  let wrapper = obj
    .downcast_ref::<glib::BoxedAnyObject>()
    .unwrap();
  wrapper.borrow()
}

fn get_mut_value<T: 'static>(obj: &glib::Object) -> RefMut<'_, T> {
  let wrapper = obj
    .downcast_ref::<glib::BoxedAnyObject>()
    .unwrap();
  wrapper.borrow_mut()
}

pub trait RelmSelectionExt: IsA<gtk::SelectionModel> {
  fn new_model(model: gio::ListModel) -> Self;
  fn set_list_model(&mut self, model: &gio::ListModel);
}

macro_rules! impl_selection (
  ($ty:ty) => {
    impl RelmSelectionExt for $ty {
      fn new_model(model: gio::ListModel) -> Self {
        Self::new(Some(model))
      }
      fn set_list_model(&mut self, model: &gio::ListModel) {
        self.set_model(Some(model));
      }
    }
  }
);

impl_selection!(gtk::SingleSelection);
impl_selection!(gtk::MultiSelection);
impl_selection!(gtk::NoSelection);

/// An item of a [`TypedListView`].
pub trait RelmListItem: Any {
  /// The top-level widget for the list item.
  type Root: IsA<gtk::Widget>;

  /// The widgets created for the list item.
  type Widgets;

  /// Construct the widgets.
  fn setup(list_item: &gtk::ListItem) -> (Self::Root, Self::Widgets);

  /// Bind the widgets to match the data of the list item.
  fn bind(&mut self, _widgets: &mut Self::Widgets, _root: &mut Self::Root) {}

  /// Undo the steps of [`RelmListItem::bind()`] if necessary.
  fn unbind(&mut self, _widgets: &mut Self::Widgets, _root: &mut Self::Root) {}

  /// Undo the steps of [`RelmListItem::setup()`] if necessary.
  fn teardown(_list_item: &gtk::ListItem) {}
}

/// A high-level wrapper around [`gio::ListStore`],
/// [`gtk::SignalListItemFactory`] and [`gtk::ListView`].
///
/// [`TypedListView`] aims at keeping nearly the same functionality and
/// flexibility of the raw bindings while introducing a more idiomatic
/// and type-safe interface.
pub struct TypedListView<T, S> {
  /// The internal list view.
  pub view: gtk::ListView,
  /// The internal selection model.
  pub selection_model: S,
  store: gio::ListStore,
  filters: Vec<Filter>,
  sorter: Option<gtk::CustomSorter>,
  base_model: gio::ListModel,
  active_model: gio::ListModel,
  _ty: PhantomData<*const T>,
}

impl<T: std::fmt::Debug, S: std::fmt::Debug> std::fmt::Debug
  for TypedListView<T, S>
{
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TypedListView")
      .field("store", &self.store)
      .field("view", &self.view)
      .field("filters", &"<Vec<gtk::Filter>>")
      .field("base_model", &self.base_model)
      .field("selection_model", &self.selection_model)
      .finish()
  }
}

// pub type OrdFn<T> = Option<Box<dyn Fn(&T, &T) -> std::cmp::Ordering>>;

struct Filter {
  filter: gtk::CustomFilter,
  model: gtk::FilterListModel,
}

/// And item of a [`list::TypedListView`].
///
/// The interface is very similar to [`std::cell::RefCell`].
/// Ownership is calculated at runtime, so you have to borrow the
/// value explicitly which might panic if done incorrectly.
#[derive(Debug, Clone)]
pub struct TypedListItem<T> {
  inner: glib::BoxedAnyObject,
  _ty: PhantomData<*const T>,
}

impl<T: 'static> TypedListItem<T> {
  fn new(inner: glib::BoxedAnyObject) -> Self {
    Self {
      inner,
      _ty: PhantomData,
    }
  }

  // rustdoc-stripper-ignore-next
  /// Immutably borrows the wrapped value.
  ///
  /// The borrow lasts until the returned `Ref` exits scope. Multiple
  /// immutable borrows can be taken out at the same time.
  ///
  /// # Panics
  ///
  /// Panics if the value is currently mutably borrowed.
  ///
  /// For a non-panicking variant, use
  /// [`try_borrow`](#method.try_borrow).
  #[must_use]
  pub fn borrow(&self) -> Ref<'_, T> {
    self.inner.borrow()
  }

  // rustdoc-stripper-ignore-next
  /// Mutably borrows the wrapped value.
  ///
  /// The borrow lasts until the returned `RefMut` or all `RefMut`s derived
  /// from it exit scope. The value cannot be borrowed while this borrow is
  /// active.
  ///
  /// # Panics
  ///
  /// Panics if the value is currently borrowed.
  ///
  /// For a non-panicking variant, use
  /// [`try_borrow_mut`](#method.try_borrow_mut).
  #[must_use]
  pub fn borrow_mut(&self) -> RefMut<'_, T> {
    self.inner.borrow_mut()
  }
}

impl<T, S> TypedListView<T, S>
where
  T: RelmListItem,
  S: RelmSelectionExt,
{
  #[must_use]
  pub fn new() -> Self {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
      let list_item = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem");

      let (root, widgets) = T::setup(list_item);
      unsafe { root.set_data("widgets", widgets) };
      list_item.set_child(Some(&root));
    });

    factory.connect_bind(move |_, list_item| {
      let list_item = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem");

      let widget = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem")
        .child();

      let obj = list_item.item().unwrap();
      let mut obj = get_mut_value::<T>(&obj);

      let mut root = widget.and_downcast::<T::Root>().unwrap();

      let mut widgets = unsafe { root.steal_data("widgets") }.unwrap();
      obj.bind(&mut widgets, &mut root);
      unsafe { root.set_data("widgets", widgets) };
    });

    factory.connect_unbind(move |_, list_item| {
      let list_item = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem");

      let widget = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem")
        .child();

      let obj = list_item.item().unwrap();
      let mut obj = get_mut_value::<T>(&obj);

      let mut root = widget.and_downcast::<T::Root>().unwrap();

      let mut widgets = unsafe { root.steal_data("widgets") }.unwrap();
      obj.unbind(&mut widgets, &mut root);
      unsafe { root.set_data("widgets", widgets) };
    });

    factory.connect_teardown(move |_, list_item| {
      let list_item = list_item
        .downcast_ref::<gtk::ListItem>()
        .expect("Needs to be ListItem");

      T::teardown(list_item);
    });

    let base_model = store.clone().upcast::<gio::ListModel>();
    let base_model = gtk::SortListModel::new(
      Some(base_model.clone()),
      None::<gtk::CustomSorter>,
    )
    .upcast::<gio::ListModel>();

    let selection_model = S::new_model(base_model.clone());
    let view = gtk::ListView::new(Some(selection_model.clone()), Some(factory));

    Self {
      store,
      view,
      filters: Vec::new(),
      sorter: None,
      active_model: base_model.clone(),
      base_model,
      selection_model,
      _ty: PhantomData,
    }
  }

  /// Add a function to filter the stored items.
  /// Returning `false` will simply hide the item.
  ///
  /// Note that several filters can be added on top of each other.
  pub fn add_filter<F: Fn(&T) -> bool + 'static>(&mut self, f: F) {
    let filter = gtk::CustomFilter::new(move |obj| {
      let value = get_value::<T>(obj);
      f(&value)
    });
    let filter_model =
      gtk::FilterListModel::new(None::<gio::ListModel>, Some(filter.clone()));

    self.filters.push(Filter {
      filter,
      model: filter_model,
    });

    self.update_model();
    self
      .selection_model
      .set_list_model(&self.active_model);
  }

  /// Returns the amount of filters that were added.
  pub fn filters_len(&self) -> usize {
    self.filters.len()
  }

  /// Set a certain filter as active or inactive.
  pub fn set_filter_status(&mut self, idx: usize, active: bool) -> bool {
    if let Some(filter) = self.filters.get(idx) {
      if active {
        filter.model.set_filter(Some(&filter.filter));
      } else {
        filter
          .model
          .set_filter(None::<&gtk::CustomFilter>);
      }
      true
    } else {
      false
    }
  }

  /// Notify that a certain filter has changed.
  /// This causes the filter expression to be re-evaluated.
  ///
  /// Returns true if a filter was notified.
  pub fn notify_filter_changed(&self, idx: usize) -> bool {
    if let Some(filter) = self.filters.get(idx) {
      filter
        .filter
        .changed(gtk::FilterChange::Different);
      true
    } else {
      false
    }
  }

  /// Remove the last filter.
  pub fn pop_filter(&mut self) {
    // let filter = self.filters.pop();
    // if let Some(filter) = filter {
    //   self.active_model = filter.model.model().unwrap();
    //   self
    //     .selection_model
    //     .set_list_model(&self.active_model);
    // }
  }

  /// Remove all filters.
  pub fn clear_filters(&mut self) {
    self.filters.clear();
    self.update_model();
  }

  /// Add a new item at the end of the list.
  pub fn append(&mut self, value: T) {
    self
      .store
      .append(&glib::BoxedAnyObject::new(value));
  }

  /// Add new items from an iterator the the end of the list.
  pub fn extend_from_iter<I: IntoIterator<Item = T>>(&mut self, init: I) {
    let objects: Vec<glib::BoxedAnyObject> = init
      .into_iter()
      .map(glib::BoxedAnyObject::new)
      .collect();
    self.store.extend_from_slice(&objects);
  }

  /// Find the index of the first item that matches a certain function.
  pub fn find<F: FnMut(&T) -> bool>(&self, mut equal_func: F) -> Option<u32> {
    self.store.find_with_equal_func(move |obj| {
      let value = get_value::<T>(obj);
      equal_func(&value)
    })
  }

  /// Returns true if the list is empty.
  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }

  /// Returns the length of the list (without filters).
  pub fn len(&self) -> u32 {
    self.store.n_items()
  }

  /// Get the [`TypedListItem`] at the specified position.
  ///
  /// Returns [`None`] if the position is invalid.
  pub fn get(&self, position: u32) -> Option<TypedListItem<T>> {
    if let Some(obj) = self.store.item(position) {
      let wrapper = obj.downcast::<glib::BoxedAnyObject>().unwrap();
      Some(TypedListItem::new(wrapper))
    } else {
      None
    }
  }

  /// Get the visible [`TypedListItem`] at the specified position,
  /// (the item at the given position after filtering and sorting).
  ///
  /// Returns [`None`] if the position is invalid.
  pub fn get_visible(&self, position: u32) -> Option<TypedListItem<T>> {
    if let Some(obj) = self.active_model.item(position) {
      let wrapper = obj.downcast::<glib::BoxedAnyObject>().unwrap();
      Some(TypedListItem::new(wrapper))
    } else {
      None
    }
  }

  /// Insert an item at a specific position.
  pub fn insert(&mut self, position: u32, value: T) {
    self
      .store
      .insert(position, &glib::BoxedAnyObject::new(value));
  }

  /// Insert an item into the list and calculate its position from
  /// a sorting function.
  pub fn insert_sorted<F: FnMut(&T, &T) -> std::cmp::Ordering>(
    &self,
    value: T,
    mut compare_func: F,
  ) -> u32 {
    let item = glib::BoxedAnyObject::new(value);

    let compare = move |first: &glib::Object,
                        second: &glib::Object|
          -> std::cmp::Ordering {
      let first = get_value::<T>(first);
      let second = get_value::<T>(second);
      compare_func(&first, &second)
    };

    self.store.insert_sorted(&item, compare)
  }

  /// Remove an item at a specific position.
  pub fn remove(&mut self, position: u32) {
    self.store.remove(position);
  }

  /// Remove all items.
  pub fn clear(&mut self) {
    self.store.remove_all();
  }

  pub fn set_sorter<F>(&mut self, f: F)
  where
    F: Fn(&T, &T) -> std::cmp::Ordering + 'static,
  {
    let sorter = gtk::CustomSorter::new(move |first, second| {
      let first = get_value::<T>(first);
      let second = get_value::<T>(second);
      match f(&first, &second) {
        std::cmp::Ordering::Less => gtk::Ordering::Smaller,
        std::cmp::Ordering::Equal => gtk::Ordering::Equal,
        std::cmp::Ordering::Greater => gtk::Ordering::Larger,
      }
    });

    self.sorter = Some(sorter);
  }

  fn update_model(&mut self) {
    let mut model = self.base_model.clone();
    if let Some(sorter) = &self.sorter {
      let sort_model =
        gtk::SortListModel::new(Some(model), Some(sorter.clone()));
      model = sort_model.upcast::<gio::ListModel>();
    }

    for filter in &self.filters {
      filter.model.set_model(Some(&model));
      model = filter
        .model
        .upcast_ref::<gio::ListModel>()
        .clone();
    }

    self.active_model = model.upcast::<gio::ListModel>();
    self
      .selection_model
      .set_list_model(&self.active_model);
  }
}
