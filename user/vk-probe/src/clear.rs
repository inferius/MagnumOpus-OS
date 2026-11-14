//! Krok 2 a 3: logicke zarizeni, pamet, obrazek a command buffer.
//!
//! Obrazek 64x64 `R8G8B8A8_UNORM` (pamet zarizeni) se vyplni barvou
//! (`vkCmdClearColorImage`), zkopiruje do bufferu v pameti viditelne
//! z hosta (`HOST_VISIBLE | HOST_COHERENT`) a buffer se precte pres blob
//! zdroj z jeho `VkDeviceMemory` (namapovany do procesu). Vsechny pixely
//! musi mit vyplnenou barvu.

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use libos::time;
use venus::REPLY;
use venus::client::{Error, Venus, check};
use venus::vk::{self, cmd, *};

const W: u32 = 64;
const H: u32 = 64;
/// Barva vyplne a ocekavane bajty (R, G, B, A).
const COLOR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
const EXPECT: [u8; 4] = [64, 128, 191, 255];

/// Prvni typ pameti z `bits` se vsemi priznaky `want`.
fn memory_type(m: &VkPhysicalDeviceMemoryProperties, bits: u32, want: u32) -> Option<u32> {
    (0..m.memory_type_count.min(32))
        .find(|&i| bits & (1 << i) != 0 && m.memory_types[i as usize].property_flags & want == want)
}

fn whole_image() -> VkImageSubresourceRange {
    VkImageSubresourceRange {
        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

fn image_barrier(image: VkImage, old: VkImageLayout, new: VkImageLayout, src: u32, dst: u32) -> VkImageMemoryBarrier {
    VkImageMemoryBarrier {
        src_access_mask: src,
        dst_access_mask: dst,
        old_layout: old,
        new_layout: new,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image,
        subresource_range: whole_image(),
    }
}

/// Bariera jen s obrazky.
fn barrier(e: &mut venus::cs::Encoder, cb: VkCommandBuffer, src: u32, dst: u32, b: &[VkImageMemoryBarrier]) {
    cmd::cmd_pipeline_barrier(e, 0, cb, src, dst, 0, 0, &[], 0, &[], b.len() as u32, b);
}

/// Vyplni obrazek, precte ho zpet a overi pixely. Vraci radek vysledku.
pub fn run(v: &mut Venus, instance_pd: VkPhysicalDevice) -> Result<String, Error> {
    let started = time::now();
    let pd = instance_pd;
    // Rodina front s grafikou nebo vypocty (vkCmdClearColorImage).
    let q = v.call(
        |e| cmd::get_physical_device_queue_family_properties(e, REPLY, pd, Some(0), false),
        cmd::decode_get_physical_device_queue_family_properties_reply,
    )?;
    let n = q.p_queue_family_property_count.unwrap_or(0);
    let q = v.call(
        |e| cmd::get_physical_device_queue_family_properties(e, REPLY, pd, Some(n), true),
        cmd::decode_get_physical_device_queue_family_properties_reply,
    )?;
    let families = q.p_queue_family_properties.unwrap_or_default();
    let family = families
        .iter()
        .position(|f| f.queue_flags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT) != 0)
        .ok_or(Error::Vk(VK_ERROR_FEATURE_NOT_PRESENT))? as u32;
    let mem = v
        .call(|e| cmd::get_physical_device_memory_properties(e, REPLY, pd), cmd::decode_get_physical_device_memory_properties_reply)?
        .p_memory_properties
        .ok_or(Error::Vk(VK_ERROR_INITIALIZATION_FAILED))?;

    // Logicke zarizeni s jednou frontou.
    let device = VkDevice(v.new_id());
    let prio = [1.0f32];
    let qci = [VkDeviceQueueCreateInfo { queue_family_index: family, queue_count: 1, p_queue_priorities: &prio, ..Default::default() }];
    let dci = VkDeviceCreateInfo { queue_create_info_count: 1, p_queue_create_infos: &qci, ..Default::default() };
    check(v.call(|e| cmd::create_device(e, REPLY, pd, &dci, device), cmd::decode_create_device_reply)?.ret)?;
    let queue = VkQueue(v.new_id());
    // Renderer chce frontu svazanou s casovou osou (ring_idx 1..63).
    let timeline = VkDeviceQueueTimelineInfoMESA { ring_idx: 1 };
    let chain = [Next::VkDeviceQueueTimelineInfoMESA(&timeline)];
    let qi = VkDeviceQueueInfo2 { p_next: &chain, flags: 0, queue_family_index: family, queue_index: 0 };
    v.send(|e| cmd::get_device_queue2(e, 0, device, &qi, queue))?;

    // Obrazek v pameti zarizeni.
    let image = VkImage(v.new_id());
    let ici = VkImageCreateInfo {
        image_type: VK_IMAGE_TYPE_2D,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        extent: VkExtent3D { width: W, height: H, depth: 1 },
        mip_levels: 1,
        array_layers: 1,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_OPTIMAL,
        usage: VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        ..Default::default()
    };
    check(v.call(|e| cmd::create_image(e, REPLY, device, &ici, image), cmd::decode_create_image_reply)?.ret)?;
    let req = v
        .call(|e| cmd::get_image_memory_requirements(e, REPLY, device, image), cmd::decode_get_image_memory_requirements_reply)?
        .p_memory_requirements
        .ok_or(Error::Vk(VK_ERROR_INITIALIZATION_FAILED))?;
    let ty = memory_type(&mem, req.memory_type_bits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)
        .or_else(|| memory_type(&mem, req.memory_type_bits, 0))
        .ok_or(Error::Vk(VK_ERROR_OUT_OF_DEVICE_MEMORY))?;
    let image_mem = VkDeviceMemory(v.new_id());
    let mai = VkMemoryAllocateInfo { allocation_size: req.size, memory_type_index: ty };
    check(v.call(|e| cmd::allocate_memory(e, REPLY, device, &mai, image_mem), cmd::decode_allocate_memory_reply)?.ret)?;
    check(v.call(|e| cmd::bind_image_memory(e, REPLY, device, image, image_mem, 0), cmd::decode_bind_image_memory_reply)?.ret)?;

    // Buffer v pameti viditelne z hosta (cteni pres blob).
    let size = (W * H * 4) as u64;
    let buffer = VkBuffer(v.new_id());
    let bci = VkBufferCreateInfo {
        size,
        usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        ..Default::default()
    };
    check(v.call(|e| cmd::create_buffer(e, REPLY, device, &bci, buffer), cmd::decode_create_buffer_reply)?.ret)?;
    let req = v
        .call(|e| cmd::get_buffer_memory_requirements(e, REPLY, device, buffer), cmd::decode_get_buffer_memory_requirements_reply)?
        .p_memory_requirements
        .ok_or(Error::Vk(VK_ERROR_INITIALIZATION_FAILED))?;
    let host = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    let ty = memory_type(&mem, req.memory_type_bits, host).ok_or(Error::Vk(VK_ERROR_OUT_OF_HOST_MEMORY))?;
    let alloc_size = req.size.next_multiple_of(4096);
    let buf_mem = VkDeviceMemory(v.new_id());
    let mai = VkMemoryAllocateInfo { allocation_size: alloc_size, memory_type_index: ty };
    check(v.call(|e| cmd::allocate_memory(e, REPLY, device, &mai, buf_mem), cmd::decode_allocate_memory_reply)?.ret)?;
    check(v.call(|e| cmd::bind_buffer_memory(e, REPLY, device, buffer, buf_mem, 0), cmd::decode_bind_buffer_memory_reply)?.ret)?;
    // Pamet bufferu jako blob namapovany do procesu; predem vyplnit, at je
    // poznat, ze ji GPU opravdu prepsalo.
    let blob = v.create_blob(buf_mem.0, alloc_size)?;
    blob.map.write_bytes(0, &alloc::vec![0xA5u8; size as usize]);

    // Command buffer: bariera, vyplneni, bariera, kopie do bufferu.
    let pool = VkCommandPool(v.new_id());
    let pci = VkCommandPoolCreateInfo { flags: 0, queue_family_index: family };
    check(v.call(|e| cmd::create_command_pool(e, REPLY, device, &pci, pool), cmd::decode_create_command_pool_reply)?.ret)?;
    let cb = VkCommandBuffer(v.new_id());
    let cai = VkCommandBufferAllocateInfo { command_pool: pool, level: VK_COMMAND_BUFFER_LEVEL_PRIMARY, command_buffer_count: 1 };
    check(v.call(|e| cmd::allocate_command_buffers(e, REPLY, device, &cai, Some(&[cb])), cmd::decode_allocate_command_buffers_reply)?.ret)?;
    let fence = VkFence(v.new_id());
    check(v.call(|e| cmd::create_fence(e, REPLY, device, &VkFenceCreateInfo { flags: 0 }, fence), cmd::decode_create_fence_reply)?.ret)?;
    let begin = VkCommandBufferBeginInfo { flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT, p_inheritance_info: None };
    let to_dst = [image_barrier(image, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 0, VK_ACCESS_TRANSFER_WRITE_BIT)];
    let to_src = [image_barrier(
        image,
        VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        VK_ACCESS_TRANSFER_WRITE_BIT,
        VK_ACCESS_TRANSFER_READ_BIT,
    )];
    let to_host = [VkBufferMemoryBarrier {
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_HOST_READ_BIT,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        buffer,
        offset: 0,
        size,
    }];
    let range = [whole_image()];
    let copy = [VkBufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: VkImageSubresourceLayers { aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT, mip_level: 0, base_array_layer: 0, layer_count: 1 },
        image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
        image_extent: VkExtent3D { width: W, height: H, depth: 1 },
    }];
    let cbs = [cb];
    let submit = [VkSubmitInfo { command_buffer_count: 1, p_command_buffers: &cbs, ..Default::default() }];
    let r = v.call(
        |e| {
            cmd::begin_command_buffer(e, 0, cb, &begin);
            barrier(e, cb, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, &to_dst);
            let color = VkClearColorValue::Float32(COLOR);
            cmd::cmd_clear_color_image(e, 0, cb, image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &color, 1, &range);
            barrier(e, cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, &to_src);
            cmd::cmd_copy_image_to_buffer(e, 0, cb, image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, buffer, 1, &copy);
            cmd::cmd_pipeline_barrier(
                e,
                0,
                cb,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_PIPELINE_STAGE_HOST_BIT,
                0,
                0,
                &[],
                1,
                &to_host,
                0,
                &[],
            );
            cmd::end_command_buffer(e, 0, cb);
            cmd::queue_submit(e, REPLY, queue, 1, &submit, fence);
        },
        cmd::decode_queue_submit_reply,
    )?;
    check(r.ret)?;
    // Plot fronty (osa 1) jako plot jadra, bez dotazovani vkGetFenceStatus.
    v.wait_queue(1, Duration::from_secs(20))?;

    // Pixely zpet z blobu.
    let mut px = alloc::vec![0u8; size as usize];
    blob.map.read_bytes(0, &mut px);
    let bad = px
        .chunks(4)
        .filter(|p| p.iter().zip(EXPECT).any(|(&a, b)| a.abs_diff(b) > 1))
        .count();
    let first = [px[0], px[1], px[2], px[3]];
    let map_info = blob.map_info;
    let ms = (time::now() - started) / 1_000_000;
    // Uklid (bloby pred uvolnenim pameti).
    v.destroy_blob(blob)?;
    v.send(|e| {
        cmd::destroy_fence(e, 0, device, fence);
        cmd::free_command_buffers(e, 0, device, pool, 1, &cbs);
        cmd::destroy_command_pool(e, 0, device, pool);
        cmd::destroy_buffer(e, 0, device, buffer);
        cmd::free_memory(e, 0, device, buf_mem);
        cmd::destroy_image(e, 0, device, image);
        cmd::free_memory(e, 0, device, image_mem);
        cmd::destroy_device(e, 0, device);
    })?;
    if bad != 0 {
        libos::println!("vk-probe: {bad} pixelu ma spatnou barvu, prvni {first:?}");
        return Err(Error::Vk(vk::VK_ERROR_UNKNOWN));
    }
    Ok(format!(
        "ok {W}x{H} {:02x}{:02x}{:02x}{:02x} map_info={map_info} {ms} ms",
        first[0], first[1], first[2], first[3]
    ))
}
