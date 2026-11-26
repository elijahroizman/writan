#[derive(Clone, Copy, Debug)]
pub(crate) struct Color(pub [f32; 4]);

impl Color {
    pub const BLACK: Self = Self([0., 0., 0., 1.]);

    pub fn parse(value: &str) -> Option<Self> {
        let color: csscolorparser::Color = value.trim().parse().ok()?;
        Some(Self(color.to_array()))
    }

    pub fn alpha(self) -> f32 {
        self.0[3]
    }

    pub fn with_opacity(self, opacity: f32) -> Self {
        let [r, g, b, a] = self.0;
        Self([r, g, b, a * opacity])
    }
}

pub(crate) struct Gradient {
    pub direction: [f64; 2],
    pub stops: Vec<(f64, Color)>,
}

fn direction(value: &str, width: f64, height: f64) -> Option<[f64; 2]> {
    if let Some(sides) = value.strip_prefix("to ") {
        let (mut x, mut y) = (0., 0.);
        for side in sides.split_whitespace() {
            match side {
                "left" => x = -1.,
                "right" => x = 1.,
                "top" => y = -1.,
                "bottom" => y = 1.,
                _ => return None,
            }
        }
        if x != 0. && y != 0. {
            return Some([x * height, y * width]);
        }
        return Some([x, y]);
    }
    for (unit, factor) in
        [("deg", std::f64::consts::PI / 180.), ("grad", std::f64::consts::PI / 200.), ("rad", 1.), ("turn", std::f64::consts::TAU)]
    {
        if let Some(number) = value.strip_suffix(unit) {
            let angle = number.parse::<f64>().ok()? * factor;
            return Some([angle.sin(), -angle.cos()]);
        }
    }
    None
}

fn split_arguments(value: &str) -> Vec<&str> {
    let mut start = 0;
    let mut depth: usize = 0;
    let mut args = Vec::new();
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(value[start..].trim());
    args
}

fn stop(value: &str, length: f64) -> Option<(Color, Vec<f64>)> {
    if let Some(color) = Color::parse(value) {
        return Some((color, Vec::new()));
    }
    let color_end = value.rfind(')').map(|i| i + 1).or_else(|| value.find(char::is_whitespace))?;
    let color = Color::parse(&value[..color_end])?;
    let positions = value[color_end..]
        .split_whitespace()
        .map(|position| {
            if let Some(percent) = position.strip_suffix('%') {
                Some(percent.parse::<f64>().ok()? / 100.)
            } else if let Some(pixels) = position.strip_suffix("px") {
                Some(pixels.parse::<f64>().ok()? / length)
            } else if position == "0" {
                Some(0.)
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>()?;
    positions.iter().all(|x| x.is_finite()).then_some((color, positions))
}

impl Gradient {
    pub fn parse(value: &str, width: f64, height: f64) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        let args = split_arguments(value.strip_prefix("linear-gradient(")?.strip_suffix(')')?);
        let parsed_direction = direction(args[0], width, height);
        let [dx, dy] = parsed_direction.unwrap_or([0., 1.]);
        let norm = dx.hypot(dy);
        if norm == 0. || !norm.is_finite() {
            return None;
        }
        let direction = [dx / norm, dy / norm];
        let length = direction[0].abs() * width + direction[1].abs() * height;
        if length <= 0. {
            return None;
        }
        let mut stops = Vec::new();
        for arg in &args[usize::from(parsed_direction.is_some())..] {
            let (color, offsets) = stop(arg, length)?;
            if offsets.is_empty() {
                stops.push((None, color));
            } else {
                stops.extend(offsets.into_iter().map(|offset| (Some(offset), color)));
            }
        }
        if stops.len() < 2 {
            return None;
        }
        let last = stops.len() - 1;
        stops[0].0.get_or_insert(0.);
        stops[last].0.get_or_insert(1.);
        let mut first = 0;
        for end in 1..stops.len() {
            if let Some(offset) = stops[end].0 {
                let start = stops[first].0?;
                let offset = offset.max(start);
                for (i, stop) in stops[first + 1..=end].iter_mut().enumerate() {
                    stop.0 = Some(start + (offset - start) * (i + 1) as f64 / (end - first) as f64);
                }
                first = end;
            }
        }
        Some(Self { direction, stops: stops.into_iter().filter_map(|(offset, color)| Some((offset?, color))).collect() })
    }
}
