use crate::host::{Error, Host};

use crate::layout::{PARAMS, STATE};

use crate::resident::{Groups, Resident};

use crate::rule::{FIELD, Kind, POINT, Pass, ROOM, SLOPE, SPARE, TOLD_SWING, TOLD_TURNING, built};

use crate::sand::Grid;

pub struct Board<'host> {
    pub resident: Resident<'host>,
    pub over: Groups,
    pub grid: Grid,
    pub depth: u32,
    pub angle: (u32, u32),
    pub loud: u32,
    pub planted: bool,
}

pub fn seated(
    host: &Host,
    grid: Grid,
    kind: Kind,
    angle: (u32, u32),
    depth: u32,
) -> Result<Board<'_>, Error> {
    let width = host.width();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let passes = [
        built(width, grid, Pass::Ahead, kind)?,
        built(width, grid, Pass::Back, kind)?,
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let resident = host.resident(&sizes, &modules)?;
    Ok(Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth,
        angle,
        loud: 1,
        planted: false,
    })
}

impl Board<'_> {
    pub fn tune(&mut self, turning: u32, swing: u32) -> Result<(), Error> {
        self.angle = (turning, swing);
        let field = vec![turning; self.grid.cells()];
        self.resident.write(FIELD as usize, &field)?;
        self.planted = true;
        Ok(())
    }

    pub fn sow(&mut self, field: &[u32]) -> Result<(), Error> {
        self.resident.write(FIELD as usize, field)?;
        self.planted = true;
        Ok(())
    }

    pub fn run(&mut self, cells: &[u32]) -> Result<Vec<u32>, Error> {
        if !self.planted {
            let field = vec![self.angle.0; self.grid.cells()];
            self.resident.write(FIELD as usize, &field)?;
            self.planted = true;
        }
        self.resident.write(STATE as usize, cells)?;
        self.resident
            .write_at(PARAMS as usize, 4, &[self.angle.0, self.angle.1])?;
        let ahead = [self.over, Groups::Fixed(0)];
        let stages: Vec<&[Groups]> = vec![&ahead];
        for frame in 0..self.depth {
            self.resident.write_at(PARAMS as usize, 0, &[frame])?;
            self.resident.staged(&stages)?;
        }
        self.resident.read(STATE as usize, self.grid.cells())
    }

    pub fn slope(&mut self, cells: &[u32], at: usize) -> Result<Vec<u32>, Error> {
        let mut weight = vec![0_u32; self.grid.cells()];
        let held = weight.get_mut(at).ok_or(Error::Overrun {
            index: at,
            needed: at + 1,
            held: self.grid.cells(),
        })?;
        *held = 1 << POINT;
        Ok(self.carry(cells, &weight)?.0)
    }

    pub fn wipe(&mut self) -> Result<(), Error> {
        let blank = vec![0_u32; self.grid.cells()];
        self.resident.write(TOLD_TURNING as usize, &blank)?;
        self.resident.write(TOLD_SWING as usize, &blank)?;
        Ok(())
    }

    pub fn told_each(&mut self) -> Result<Vec<i32>, Error> {
        Ok(self
            .resident
            .read(TOLD_TURNING as usize, self.grid.cells())?
            .iter()
            .map(|one| *one as i32)
            .collect())
    }

    pub fn told(&mut self) -> Result<(f64, f64), Error> {
        let piled = |held: Vec<u32>| held.iter().map(|one| f64::from(*one as i32)).sum::<f64>();
        let turning = self
            .resident
            .read(TOLD_TURNING as usize, self.grid.cells())?;
        let swing = self.resident.read(TOLD_SWING as usize, self.grid.cells())?;
        let back = f64::from(1_u32 << SPARE) * f64::from(self.loud);
        Ok((piled(turning) / back, piled(swing) / back))
    }

    pub fn carry(&mut self, cells: &[u32], weight: &[u32]) -> Result<(Vec<u32>, Vec<u32>), Error> {
        let ended = self.run(cells)?;
        self.wipe()?;
        let peak = weight
            .iter()
            .map(|one| (*one as i32).unsigned_abs())
            .max()
            .unwrap_or(1)
            .max(1);
        let room = (ROOM / peak).max(1);
        self.loud = 1_u32 << (31 - room.leading_zeros());
        let louder: Vec<u32> = weight
            .iter()
            .map(|one| ((*one as i32).wrapping_mul(self.loud as i32)) as u32)
            .collect();
        let weight = &louder;
        self.resident.write(STATE as usize, &ended)?;
        self.resident
            .write_at(PARAMS as usize, 4, &[self.angle.0, self.angle.1])?;
        self.resident.write(SLOPE as usize, weight)?;

        let back = [Groups::Fixed(0), self.over];
        let stages: Vec<&[Groups]> = vec![&back];
        for frame in (0..self.depth).rev() {
            self.resident.write_at(PARAMS as usize, 0, &[frame])?;
            self.resident.staged(&stages)?;
        }
        let home = self.resident.read(STATE as usize, self.grid.cells())?;
        if let Some((at, (was, now))) = home
            .iter()
            .zip(cells)
            .enumerate()
            .find(|(_, (home, cell))| home != cell)
            .map(|(at, (home, cell))| (at, (*cell, *home)))
        {
            return Err(Error::Strayed { at, was, now });
        }
        let carried = self
            .resident
            .read(SLOPE as usize, self.grid.cells())?
            .iter()
            .map(|one| ((*one as i32) / self.loud as i32) as u32)
            .collect();
        Ok((carried, ended))
    }
}
