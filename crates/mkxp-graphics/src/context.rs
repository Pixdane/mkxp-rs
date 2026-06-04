/// 混合模式。
#[derive(Debug, Clone, Copy)]
pub enum BlendMode {
    Normal,
    Addition,
    Subtraction,
}

/// 混合模式栈。
pub struct BlendStack {
    stack: Vec<BlendMode>,
}

impl Default for BlendStack {
    fn default() -> Self {
        Self::new()
    }
}

impl BlendStack {
    pub fn new() -> Self {
        Self {
            stack: vec![BlendMode::Normal],
        }
    }

    pub fn push(&mut self, mode: BlendMode) {
        self.stack.push(mode);
    }

    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

/// 裁剪区域栈。
pub struct ScissorStack {
    stack: Vec<Option<(u32, u32, u32, u32)>>,
}

impl Default for ScissorStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ScissorStack {
    pub fn new() -> Self {
        Self { stack: vec![None] }
    }

    pub fn push(&mut self, x: u32, y: u32, w: u32, h: u32) {
        let current = self.stack.last().copied().flatten();
        let intersected = match current {
            None => Some((x, y, w, h)),
            Some((cx, cy, cw, ch)) => {
                let ix = cx.max(x);
                let iy = cy.max(y);
                let ir = (cx + cw).min(x + w);
                let ib = (cy + ch).min(y + h);
                if ir > ix && ib > iy {
                    Some((ix, iy, ir - ix, ib - iy))
                } else {
                    Some((0, 0, 0, 0))
                }
            }
        };
        self.stack.push(intersected);
    }

    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

/// 渲染操作上下文。
///
/// 每个帧合成时创建，携带所有元素画自己需要的资源。
/// 当前为骨架实现。
pub struct DrawContext<'a> {
    pub blend: BlendStack,
    pub scissor: ScissorStack,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> Default for DrawContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> DrawContext<'a> {
    pub fn new() -> Self {
        Self {
            blend: BlendStack::new(),
            scissor: ScissorStack::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}
