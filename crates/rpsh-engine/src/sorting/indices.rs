//! Ordering algorithms for the full-sort and bounded-heap parameter sets.
use rpsh_runtime::RuntimeError;
use std::cmp::Ordering;

pub(super) fn ordered(
    len: usize,
    compare: &Compare<'_>,
    stable: bool,
    unique: bool,
    limit: Option<usize>,
    top: bool,
) -> Result<Vec<usize>, RuntimeError> {
    let mut items: Vec<_> = (0..len).collect();
    let indexed =
        |a, b| Ok(compare(a, b)?.then_with(|| if stable { a.cmp(&b) } else { Ordering::Equal }));
    if let Some(capacity) = limit.filter(|&n| n < len) {
        let direction = if top {
            Ordering::Less
        } else {
            Ordering::Greater
        };
        let used = bounded_heap(&mut items, capacity, direction, unique, compare, &indexed)?;
        items.truncate(used);
        index_sort(&mut items, &indexed)?;
        return Ok(items);
    }
    index_sort(&mut items, &indexed)?;
    if unique {
        let mut used = 0;
        for i in 0..items.len() {
            if used == 0 || !compare(items[used - 1], items[i])?.is_eq() {
                items[used] = items[i];
                used += 1;
            }
        }
        items.truncate(used);
    }
    Ok(items)
}

fn insert_unique(
    seen: &mut Vec<usize>,
    key: usize,
    compare: &Compare<'_>,
) -> Result<bool, RuntimeError> {
    let mut left = 0;
    let mut right = seen.len();
    while left < right {
        let at = left + (right - left) / 2;
        match compare(key, seen[at])? {
            Ordering::Less => right = at,
            Ordering::Greater => left = at + 1,
            Ordering::Equal => return Ok(false),
        }
    }
    seen.insert(left, key);
    Ok(true)
}

// Upstream SortObjectCommand.Heapify checks the compacted slot against the
// heap root before reading past discarded duplicates. These two positions
// can differ, affecting which records a bounded unique sort returns.
fn bounded_heap(
    items: &mut [usize],
    capacity: usize,
    direction: Ordering,
    unique: bool,
    compare: &Compare<'_>,
    indexed: &Compare<'_>,
) -> Result<usize, RuntimeError> {
    let mut seen = Vec::new();
    let (mut size, mut write) = (0, 0);
    for read in 0..items.len() {
        if size == capacity && indexed(items[0], items[write])? == direction {
            write += 1;
            continue;
        }
        if unique && !insert_unique(&mut seen, items[read], compare)? {
            continue;
        }
        items[write] = items[read];
        let mut child = write;
        while child > 0 {
            let parent = (child.min(capacity) - 1) / 2;
            if indexed(items[child], items[parent])? == direction {
                break;
            }
            items.swap(child, parent);
            child = parent;
        }
        if size < capacity {
            size += 1;
        } else {
            items[0] = items[write];
            let mut parent = 0;
            while 2 * parent + 1 < capacity {
                let left = 2 * parent + 1;
                let right = left + 1;
                let child = if right < capacity && indexed(items[left], items[right])? == direction
                {
                    right
                } else {
                    left
                };
                let order = indexed(items[child], items[parent])?;
                if order.is_eq() || order == direction {
                    break;
                }
                items.swap(parent, child);
                parent = child;
            }
        }
        write += 1;
    }
    Ok(size)
}

// List.Sort's median-of-three partitioning is observable when equal keys
// have different objects. Sort indices so property evaluations stay in place.
type Compare<'a> = dyn Fn(usize, usize) -> Result<Ordering, RuntimeError> + 'a;
fn index_sort(items: &mut [usize], compare: &Compare<'_>) -> Result<(), RuntimeError> {
    let depth = 2 * (usize::BITS - items.len().leading_zeros());
    intro_sort(items, depth, compare)
}

fn intro_sort(
    mut items: &mut [usize],
    mut depth: u32,
    compare: &Compare<'_>,
) -> Result<(), RuntimeError> {
    while items.len() > 1 {
        let last = items.len() - 1;
        if items.len() <= 16 {
            if items.len() == 2 || items.len() == 3 {
                swap_ordered(items, 0, 1, compare)?;
                if last == 2 {
                    swap_ordered(items, 0, 2, compare)?;
                    swap_ordered(items, 1, 2, compare)?;
                }
            } else {
                for i in 1..items.len() {
                    let pivot = items[i];
                    let mut j = i;
                    while j > 0 && compare(pivot, items[j - 1])?.is_lt() {
                        items[j] = items[j - 1];
                        j -= 1;
                    }
                    items[j] = pivot;
                }
            }
            break;
        }
        if depth == 0 {
            for root in (0..items.len() / 2).rev() {
                sift_down(items, root, compare)?;
            }
            for end in (1..items.len()).rev() {
                items.swap(0, end);
                sift_down(&mut items[..end], 0, compare)?;
            }
            break;
        }
        depth -= 1;
        let mid = last / 2;
        swap_ordered(items, 0, mid, compare)?;
        swap_ordered(items, 0, last, compare)?;
        swap_ordered(items, mid, last, compare)?;
        let pivot = items[mid];
        items.swap(mid, last - 1);
        let (mut left, mut right) = (0, last - 1);
        loop {
            left += 1;
            while left < last && compare(items[left], pivot)?.is_lt() {
                left += 1;
            }
            right -= 1;
            while right > 0 && compare(pivot, items[right])?.is_lt() {
                right -= 1;
            }
            if left >= right {
                break;
            }
            items.swap(left, right);
        }
        items.swap(left, last - 1);
        let (low, high) = items.split_at_mut(left);
        intro_sort(&mut high[1..], depth, compare)?;
        items = low;
    }
    Ok(())
}

fn swap_ordered(
    items: &mut [usize],
    a: usize,
    b: usize,
    compare: &Compare<'_>,
) -> Result<(), RuntimeError> {
    if compare(items[a], items[b])?.is_gt() {
        items.swap(a, b);
    }
    Ok(())
}

fn sift_down(
    items: &mut [usize],
    mut root: usize,
    compare: &Compare<'_>,
) -> Result<(), RuntimeError> {
    while 2 * root + 1 < items.len() {
        let mut child = 2 * root + 1;
        if child + 1 < items.len() && compare(items[child], items[child + 1])?.is_lt() {
            child += 1;
        }
        if !compare(items[root], items[child])?.is_lt() {
            break;
        }
        items.swap(root, child);
        root = child;
    }
    Ok(())
}
