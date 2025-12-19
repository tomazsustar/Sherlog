// src/user_actions.rs

use crate::log_store::LogStoreLinear;
use crate::ui_formatting;
use crate::model_internal::LogEntryExt;

use gtk::prelude::EntryExt;
use gtk::prelude::WidgetExt;
use gtk::prelude::ToggleButtonExt;

use std::rc::Rc;
use std::cell::RefCell;

use regex::Regex;

// handle search
pub fn search_changed(
		search_entry: &gtk::SearchEntry,
		case_sensitive_btn: &gtk::CheckButton,
        enable_btn: &gtk::CheckButton,
		store: &mut LogStoreLinear,
		drawing_area: &gtk::DrawingArea,
	) {
    let enabled = enable_btn.is_active();
    let search_text = search_entry.text().to_string();
    let case_sensitive = case_sensitive_btn.is_active();

    search_entry.set_sensitive(enabled);
	case_sensitive_btn.set_sensitive(enabled);

    if search_text.is_empty() || !enabled {
        if enabled  {
            log::info!("Search empty");
        } else {
            log::info!("Search disabled");
        }
        store.filter_store(
            &|_entry: &LogEntryExt| true,
            true,
            crate::model_internal::VISIBLE_OFF_FILTER,
        );
    } else {
        log::info!("search_changed {}", &search_text);
        if case_sensitive {
            store.filter_store(
                &|entry: &LogEntryExt| entry.message.contains(&search_text),
                true,
                crate::model_internal::VISIBLE_OFF_FILTER,
            );
            store.filter_store(
                &|entry: &LogEntryExt| !entry.message.contains(&search_text),
                false,
                crate::model_internal::VISIBLE_OFF_FILTER,
            );
        } else {
            // Build a case-insensitive regex for the search text
            let pattern = format!("(?i){}", regex::escape(&search_text));
            let re = Regex::new(&pattern).unwrap();

            store.filter_store(
                &|entry: &LogEntryExt| re.is_match(&entry.message),
                true,
                crate::model_internal::VISIBLE_OFF_FILTER,
            );
            store.filter_store(
                &|entry: &LogEntryExt| !re.is_match(&entry.message),
                false,
                crate::model_internal::VISIBLE_OFF_FILTER,
            );
        }
    }
    drawing_area.queue_draw();
}

// handle time shift
pub fn timeshift_changed(
entry: &gtk::Entry, 
store: &mut LogStoreLinear,
drawing_area: &gtk::DrawingArea, 
last_shift: Rc<RefCell<chrono::Duration>>, 
log_sources_to_shift: Rc<Vec<u32>>)
{
    
    // parse time shift from timeshift_entry format "+0D 00:00:00.000"
    let timeshift_text = entry.text().to_string();
    log::info!("timeshift_changed {}", &timeshift_text);
    let time_shift = ui_formatting::parse_duration(&timeshift_text);
    let actual_shift = time_shift - *last_shift.borrow();
    *last_shift.borrow_mut() = time_shift;
    // apply time shift
    
    // misuse the entry_id to remember selected
    let mut selection_active = false;
    for &offset in store.selected_single.iter() {
        if let Some(entry) = store.store.get_mut(offset) {
            entry.entry_id = 0xFFFFFFFE;
            selection_active = true;
        }
    }
    if let Some((start, end)) = store.selected_range {
        for offset in start..=end {
            if let Some(entry) = store.store.get_mut(offset) {
                entry.entry_id = 0xFFFFFFFE;
                selection_active = true;
            }
        }
    }

    // misuse the entry_id to remember the anchor
    let mut anchor_marked = false;
    if let Some(anchor_offset) = store.anchor_offset {
        if let Some(entry) = store.store.get_mut(anchor_offset) {
            entry.entry_id = 0xFFFFFFFF;
            anchor_marked = true;
        }
    }		    
                
    store.shift_store_times(&log_sources_to_shift, actual_shift);
    store.store.sort_by(|a: &LogEntryExt, b| a.timestamp.cmp(&b.timestamp));

    // After sorting, find the new anchor offset
    if anchor_marked {
        if let Some(new_offset) = store.store.iter().position(|entry| entry.entry_id == 0xFFFFFFFF) {
            store.anchor_offset = Some(new_offset);
            store.store[new_offset].entry_id = new_offset as u32; // reset if needed					
        } else {
            store.anchor_offset = None; // anchor lost
        }
    }

    // mark all selected
    if selection_active {
        store.selected_single.clear();
        store.excluded_single.clear();
        store.selected_range = None;
        for (offset, entry) in store.store.iter().enumerate() {
            if entry.entry_id == 0xFFFFFFFE {
                store.selected_single.insert(offset);
            }
        }
        // add anchor to the selection
        if let Some(anchor_offset) = store.anchor_offset {
            store.selected_single.insert(anchor_offset);
        }
    }
    
    // combination of active: false and mask: VISIBLE_ON (0) will only re calculate the visible entries and ids
    // without changing the filter state
    //   entry.visible |= mask; //apply mask
    store.filter_store(
        &|_entry: &LogEntryExt| true,
        false,
        crate::model_internal::VISIBLE_ON,
    );
    drawing_area.queue_draw();
    entry.set_text(&ui_formatting::format_duration(time_shift));
}