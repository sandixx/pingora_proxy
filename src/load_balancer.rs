use crate::backend::Backend;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use log::{info, warn};
use rand::Rng;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadBalanceStrategy {
    RoundRobin,
    Weighted,
    LeastConnections,
    StickySession,
    Random,
}

impl LoadBalanceStrategy {
    pub fn _from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "round_robin" | "round-robin" | "roundrobin" => Some(Self::RoundRobin),
            "weighted" => Some(Self::Weighted),
            "least_connections" | "least-connections" | "leastconnections" => Some(Self::LeastConnections),
            "sticky_session" | "sticky-session" | "stickysession" => Some(Self::StickySession),
            "random" => Some(Self::Random),
            _ => None,
        }
    }
}

pub struct LoadBalancer {
    pub strategy: LoadBalanceStrategy,
    pub counter: AtomicUsize,
    pub session_map: std::sync::RwLock<HashMap<String, usize>>,
}

impl LoadBalancer {
    pub fn new(strategy: LoadBalanceStrategy) -> Self {
        info!("⚖️ Load balancing strategy: {:?}", strategy);
        Self {
            strategy,
            counter: AtomicUsize::new(0),
            session_map: std::sync::RwLock::new(HashMap::new()),
        }
    }
    
    pub fn select_backend(&self, backends: &[Backend], session_id: Option<&str>) -> Option<Backend> {
        let healthy_backends: Vec<&Backend> = backends.iter().filter(|b| b.healthy).collect();
        
        if healthy_backends.is_empty() {
            warn!("⚠️ No healthy backends available, falling back to all backends");
            return self.select_from_all(backends, session_id);
        }
        
        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.round_robin(&healthy_backends),
            LoadBalanceStrategy::Weighted => self.weighted(&healthy_backends),
            LoadBalanceStrategy::LeastConnections => self.least_connections(&healthy_backends),
            LoadBalanceStrategy::StickySession => self.sticky_session(&healthy_backends, session_id),
            LoadBalanceStrategy::Random => self.random(&healthy_backends),
        }
    }
    
    fn select_from_all(&self, backends: &[Backend], session_id: Option<&str>) -> Option<Backend> {
        let all_backends: Vec<&Backend> = backends.iter().collect();
        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.round_robin(&all_backends),
            LoadBalanceStrategy::Weighted => self.weighted(&all_backends),
            LoadBalanceStrategy::LeastConnections => self.least_connections(&all_backends),
            LoadBalanceStrategy::StickySession => self.sticky_session(&all_backends, session_id),
            LoadBalanceStrategy::Random => self.random(&all_backends),
        }
    }
    
    fn round_robin(&self, backends: &[&Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }
        let index = self.counter.fetch_add(1, Ordering::Relaxed) % backends.len();
        backends.get(index).cloned().cloned()
    }
    
    fn weighted(&self, backends: &[&Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }
        
        let total_weight: usize = backends.iter().map(|b| b.weight).sum();
        if total_weight == 0 {
            return self.round_robin(backends);
        }
        
        let choice = (self.counter.fetch_add(1, Ordering::Relaxed) % 100) as usize;
        let mut acc = 0;
        
        for b in backends {
            acc += b.weight;
            if choice < acc {
                return Some((*b).clone());
            }
        }
        
        backends.first().cloned().cloned()
    }
    
    fn least_connections(&self, backends: &[&Backend]) -> Option<Backend> {
        self.round_robin(backends)
    }
    
    fn sticky_session(&self, backends: &[&Backend], session_id: Option<&str>) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }
        
        if let Some(session_id) = session_id {
            let session_map = self.session_map.read().unwrap();
            if let Some(&backend_index) = session_map.get(session_id) {
                if let Some(backend) = backends.get(backend_index) {
                    return Some((*backend).clone());
                }
            }
        }
        
        let backend_index = self.counter.fetch_add(1, Ordering::Relaxed) % backends.len();
        if let Some(session_id) = session_id {
            let mut session_map = self.session_map.write().unwrap();
            session_map.insert(session_id.to_string(), backend_index);
        }
        
        backends.get(backend_index).cloned().cloned()
    }
    
    fn random(&self, backends: &[&Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }
        
        let mut rng = rand::thread_rng();
        let index = rng.gen_range(0..backends.len());
        backends.get(index).cloned().cloned()
    }
    
    pub fn generate_session_id() -> String {
        Uuid::new_v4().to_string()
    }
}