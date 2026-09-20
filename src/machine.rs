use crate::netlist::{Chip, Netlist, TooWide};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Machine {
    pub state: u32,
    pub inputs: u32,
    pub outputs: u32,
    pub gates: u32,
    pub kinds: u32,
    pub steps: u32,
}

impl Machine {
    pub const fn over(
        state: u32,
        inputs: u32,
        outputs: u32,
        gates: u32,
        kinds: u32,
        steps: u32,
    ) -> Result<Self, TooWide> {
        let held = Self {
            state,
            inputs,
            outputs,
            gates,
            kinds,
            steps,
        };
        match held.shape() {
            Ok(_) => Ok(held),
            Err(why) => Err(why),
        }
    }

    pub const fn shape(&self) -> Result<Netlist, TooWide> {
        Netlist::over(
            self.state + self.inputs,
            self.state + self.outputs,
            self.gates,
            self.kinds,
        )
    }

    #[must_use]
    pub const fn marks(&self) -> u32 {
        self.steps * self.outputs
    }

    #[must_use]
    pub fn named(&self) -> Vec<u32> {
        let from = self.gates - (self.state + self.outputs);
        (from..self.gates).collect()
    }

    #[must_use]
    pub fn ran(&self, chip: &Chip, feed: &[u32]) -> Vec<u32> {
        let mut state = 0_u32;
        let mut said = Vec::with_capacity(self.steps as usize);
        for step in 0..self.steps as usize {
            let given = feed.get(step).copied().unwrap_or(0);
            let word = state | (given << self.state);
            let answered = chip.answers(word);
            said.push(answered >> self.state);
            state = answered & ((1 << self.state) - 1);
        }
        said
    }

    #[must_use]
    pub fn scored(&self, chip: &Chip, feed: &[u32], want: &[u32]) -> u32 {
        let said = self.ran(chip, feed);
        let past = if self.outputs >= 32 {
            0
        } else {
            u32::MAX << self.outputs
        };
        said.iter()
            .zip(want)
            .map(|(got, wanted)| (!(got ^ wanted) & !past).count_ones())
            .sum()
    }
}
