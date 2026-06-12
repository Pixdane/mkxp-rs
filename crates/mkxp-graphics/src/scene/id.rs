use slotmap::new_key_type;

new_key_type! {
    /// 场景图中节点的稳定标识符。
    ///
    /// 类比 mkxp-z 中 `SceneElement*` 指针，但不会悬空。
    /// 节点被删除后，其 NodeId 变为无效，其他节点不受影响。
    pub struct NodeId;
}
