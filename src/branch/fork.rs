use crate::layout::Windows;
use crate::live;
use crate::resident::Resident;

use super::{Failed, Refused, World, fitted};

fn copied(
    from: &mut Resident<'_>,
    into: &mut Resident<'_>,
    sizes: &[usize],
    reach: Option<(Windows, usize)>,
) -> Result<(), Failed> {
    let windows = reach.map(|(windows, _)| windows);
    for (binding, &words) in sizes.iter().enumerate() {
        let pooled = windows.is_some_and(|windows| {
            (0..windows.count()).any(|window| Windows::binding(window) as usize == binding)
        });
        if pooled {
            continue;
        }
        let held = from.read(binding, words)?;
        into.write(binding, &held)?;
    }
    if let Some((windows, reach)) = reach {
        let held = live::read_state(from, windows, reach)?;
        live::write_state(into, windows, 0, &held)?;
        drop(held);
        super::zeroed(into, &[], Some((windows, reach)))?;
    }
    Ok(())
}

impl<'host> World<'host> {
    pub fn fork(&mut self) -> Result<Self, Failed> {
        let rule = self.rule.again(self.host, &self.params)?;
        let modules: Vec<&[u32]> = self.passes.iter().map(Vec::as_slice).collect();
        let mut live = self.host.resident(&self.sizes, &modules).map_err(fitted)?;
        let reach = self.reach()?;
        copied(&mut self.live, &mut live, &self.sizes, reach)?;
        Ok(Self {
            host: self.host,
            rule,
            passes: self.passes.clone(),
            stages: self.stages.clone(),
            sizes: self.sizes.clone(),
            grid: self.grid,
            plan: self.plan,
            pool: self.pool,
            params: self.params.clone(),
            frame: self.frame,
            live,
            words_read: 0,
        })
    }

    pub fn restore(&mut self, from: &mut World<'_>) -> Result<(), Failed> {
        let unlike = |what| Err(Box::new(Refused::Unlike { what }) as Failed);
        if self.rule.name() != from.rule.name()
            || self.law().map(crate::law::Law::hash) != from.law().map(crate::law::Law::hash)
        {
            return unlike("rules");
        }
        if self.size() != from.size() {
            return unlike("sizes");
        }
        if self.pool != from.pool {
            return unlike("layouts");
        }
        if self.sizes != from.sizes || self.passes != from.passes {
            return unlike("bindings");
        }
        let reach = from.reach()?;
        copied(&mut from.live, &mut self.live, &self.sizes, reach)?;
        self.params.clone_from(&from.params);
        self.frame = from.frame;
        Ok(())
    }
}
